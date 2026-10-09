//! Receive file drops on stable winit's Wayland connection without blocking it.

#![forbid(unsafe_code)]

use std::fs::File;
use std::io::{self, Read};
use std::os::fd::OwnedFd;
use std::time::Duration;

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{Interest, Mode, PostAction, RegistrationToken};
use sctk::data_device_manager::data_device::{DataDeviceData, DataDeviceHandler};
use sctk::data_device_manager::data_offer::{DataOfferData, DataOfferHandler, DragOffer};
use sctk::data_device_manager::DataDeviceManagerState;
use sctk::globals::GlobalData;
use sctk::reexports::client::protocol::wl_data_device::WlDataDevice;
use sctk::reexports::client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use sctk::reexports::client::protocol::wl_data_offer::WlDataOffer;
use sctk::reexports::client::protocol::wl_surface::WlSurface;
use sctk::reexports::client::{Connection, Proxy, QueueHandle};

use crate::event::WindowEvent;
use crate::platform_impl::wayland::state::WinitState;
use crate::platform_impl::wayland::{make_wid, WindowId};

#[path = "drop_paths.rs"]
mod drop_paths;
use drop_paths::{paths_from_uri_list, MAX_DROP_BYTES};

const URI_LIST: &str = "text/uri-list";
const DROP_TIMEOUT: Duration = Duration::from_secs(5);

fn accepts_copy(offer: &DragOffer) -> bool {
    // Sources predating protocol v3 have implicit COPY semantics.
    offer.inner().version() < 3
        || offer.source_actions.is_empty()
        || offer.source_actions.contains(DndAction::Copy)
}

pub(crate) struct PendingDrop {
    offer: DragOffer,
    reader: RegistrationToken,
    timer: RegistrationToken,
}

impl Drop for PendingDrop {
    fn drop(&mut self) {
        self.offer.destroy();
    }
}

impl WinitState {
    fn drag_offer(&self, device: &WlDataDevice) -> Option<DragOffer> {
        device.data::<DataDeviceData>()?.drag_offer()
    }

    fn cancel_file_drop(&mut self) {
        if let Some(pending) = self.pending_drop.take() {
            self.loop_handle.remove(pending.reader);
            self.loop_handle.remove(pending.timer);
        }
    }

    fn finish_file_drop(&mut self, window: WindowId, bytes: &[u8]) {
        if let Some(pending) = self.pending_drop.take() {
            self.loop_handle.remove(pending.timer);
            let paths = paths_from_uri_list(bytes);
            if !paths.is_empty() && self.windows.borrow().contains_key(&window) {
                // The destination requests COPY only and never modifies source files.
                pending.offer.finish();
                for path in paths {
                    self.events_sink
                        .push_window_event(WindowEvent::DroppedFile(path), window);
                }
                self.dispatched_events = true;
            }
        }
    }

    fn receive_file_drop(&mut self, offer: DragOffer) -> io::Result<()> {
        self.cancel_file_drop();
        let window = make_wid(&offer.surface);
        let pipe = offer.receive(URI_LIST.to_owned())?;
        let fd: OwnedFd = pipe.into();
        let file = File::from(fd);
        let flags = rustix::fs::fcntl_getfl(&file)?;
        rustix::fs::fcntl_setfl(&file, flags | rustix::fs::OFlags::NONBLOCK)?;
        let source = Generic::new(file, Interest::READ, Mode::Level);
        let mut bytes = Vec::new();
        let reader = self
            .loop_handle
            .insert_source(source, move |_, file, state| {
                // Bound work per callback as well as the complete payload.
                let mut buffer = [0; 16384];
                for _ in 0..4 {
                    match (&**file).read(&mut buffer) {
                        Ok(0) => {
                            state.finish_file_drop(window, &bytes);
                            return Ok(PostAction::Remove);
                        }
                        Ok(len) if bytes.len() + len <= MAX_DROP_BYTES => {
                            bytes.extend_from_slice(&buffer[..len]);
                        }
                        Ok(_) => {
                            state.cancel_file_drop();
                            return Ok(PostAction::Remove);
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(_) => {
                            state.cancel_file_drop();
                            return Ok(PostAction::Remove);
                        }
                    }
                }
                Ok(PostAction::Continue)
            })
            .map_err(|error| io::Error::other(error.error))?;
        let timer = match self.loop_handle.insert_source(
            Timer::from_duration(DROP_TIMEOUT),
            |_, _, state| {
                state.cancel_file_drop();
                TimeoutAction::Drop
            },
        ) {
            Ok(timer) => timer,
            Err(error) => {
                self.loop_handle.remove(reader);
                return Err(io::Error::other(error.error));
            }
        };
        self.pending_drop = Some(PendingDrop {
            offer,
            reader,
            timer,
        });
        Ok(())
    }
}

impl DataDeviceHandler for WinitState {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        device: &WlDataDevice,
        _: f64,
        _: f64,
        surface: &WlSurface,
    ) {
        if let Some(offer) = self.drag_offer(device) {
            let supported = self.windows.borrow().contains_key(&make_wid(surface))
                && accepts_copy(&offer)
                && offer.with_mime_types(|types| types.iter().any(|mime| mime == URI_LIST));
            offer.set_actions(DndAction::Copy, DndAction::Copy);
            offer.accept_mime_type(offer.serial, supported.then(|| URI_LIST.to_owned()));
        }
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64) {}
    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, device: &WlDataDevice) {
        if let Some(offer) = self.drag_offer(device) {
            if accepts_copy(&offer)
                && self
                    .windows
                    .borrow()
                    .contains_key(&make_wid(&offer.surface))
                && offer.with_mime_types(|types| types.iter().any(|mime| mime == URI_LIST))
            {
                let cleanup = offer.clone();
                if let Err(error) = self.receive_file_drop(offer) {
                    cleanup.destroy();
                    tracing::warn!("Could not receive Wayland file drop: {error}");
                }
            }
        }
    }
}

impl DataOfferHandler for WinitState {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}

// No drag source or clipboard owner is created, so only receiving protocols need dispatch.
sctk::reexports::client::delegate_dispatch!(WinitState: [WlDataDeviceManager: GlobalData] => DataDeviceManagerState);
sctk::reexports::client::delegate_dispatch!(WinitState: [WlDataDevice: DataDeviceData] => DataDeviceManagerState);
sctk::reexports::client::delegate_dispatch!(WinitState: [WlDataOffer: DataOfferData] => DataDeviceManagerState);

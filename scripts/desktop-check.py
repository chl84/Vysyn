#!/usr/bin/env python3
"""Targeted X11 GUI check. Sends events only to the viewer process it starts."""
import ctypes as c
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import time
import zlib

root = Path(__file__).resolve().parent.parent
out = root / "artifacts"
out.mkdir(exist_ok=True)
x = c.CDLL("libX11.so.6")
display_type, window_type = c.c_void_p, c.c_ulong
x.XOpenDisplay.restype = display_type
x.XOpenDisplay.argtypes = [c.c_char_p]
x.XCloseDisplay.argtypes = [display_type]
x.XDefaultRootWindow.argtypes = [display_type]
x.XDefaultRootWindow.restype = window_type
x.XStringToKeysym.argtypes = [c.c_char_p]
x.XStringToKeysym.restype = c.c_ulong
x.XKeysymToKeycode.argtypes = [display_type, c.c_ulong]
x.XKeysymToKeycode.restype = c.c_uint

class KeyEvent(c.Structure):
    _fields_ = [("type", c.c_int), ("serial", c.c_ulong), ("send_event", c.c_int),
                ("display", display_type), ("window", window_type), ("root", window_type),
                ("subwindow", window_type), ("time", c.c_ulong), ("x", c.c_int),
                ("y", c.c_int), ("x_root", c.c_int), ("y_root", c.c_int),
                ("state", c.c_uint), ("detail", c.c_uint), ("same_screen", c.c_int)]

class Event(c.Union):
    _fields_ = [("key", KeyEvent), ("pad", c.c_long * 24)]

class XImage(c.Structure):
    _fields_ = [("width", c.c_int), ("height", c.c_int), ("xoffset", c.c_int),
                ("format", c.c_int), ("data", c.c_void_p), ("byte_order", c.c_int),
                ("bitmap_unit", c.c_int), ("bitmap_bit_order", c.c_int), ("bitmap_pad", c.c_int),
                ("depth", c.c_int), ("bytes_per_line", c.c_int), ("bits_per_pixel", c.c_int),
                ("red_mask", c.c_ulong), ("green_mask", c.c_ulong), ("blue_mask", c.c_ulong)]

x.XSendEvent.argtypes = [display_type, window_type, c.c_int, c.c_long, c.POINTER(Event)]
x.XFlush.argtypes = [display_type]
x.XSetInputFocus.argtypes = [display_type,window_type,c.c_int,c.c_ulong]
x.XGetInputFocus.argtypes = [display_type,c.POINTER(window_type),c.POINTER(c.c_int)]
x.XWarpPointer.argtypes = [display_type,window_type,window_type,c.c_int,c.c_int,c.c_uint,c.c_uint,c.c_int,c.c_int]
xt = c.CDLL('libXtst.so.6')
xt.XTestFakeButtonEvent.argtypes = [display_type,c.c_uint,c.c_int,c.c_ulong]
xt.XTestFakeKeyEvent.argtypes = [display_type,c.c_uint,c.c_int,c.c_ulong]
x.XGetGeometry.argtypes = [display_type, window_type, c.POINTER(window_type), c.POINTER(c.c_int),
    c.POINTER(c.c_int), c.POINTER(c.c_uint), c.POINTER(c.c_uint), c.POINTER(c.c_uint), c.POINTER(c.c_uint)]
x.XGetImage.argtypes = [display_type, window_type, c.c_int, c.c_int, c.c_uint, c.c_uint, c.c_ulong, c.c_int]
x.XGetImage.restype = c.POINTER(XImage)
x.XDestroyImage.argtypes = [c.POINTER(XImage)]

def guard():
    assert proc.poll() is None, 'Test viewer exited'
    if env.get('HYPRLAND_INSTANCE_SIGNATURE'):
        active = json.loads(subprocess.check_output(['hyprctl','-j','activewindow'],text=True))
        assert active.get('pid')==proc.pid, 'Test viewer lost compositor focus; input check stopped'

def event(kind, detail, px=0, py=0, state=0):
    guard()
    if kind in (4,5,6):
        x.XWarpPointer(display,0,window,0,0,0,0,px,py)
        if kind in (4,5):
            xt.XTestFakeButtonEvent(display,detail,int(kind==4),0)
        x.XFlush(display)
        return
    ev = Event()
    ev.key = KeyEvent(kind, 0, 1, display, window, x.XDefaultRootWindow(display),
                      0, 0, px, py, px, py, state, detail, 1)
    mask = {2: 1, 3: 2, 4: 4, 5: 8, 6: 64}[kind]
    assert x.XSendEvent(display, window, 0, mask, c.byref(ev))
    x.XFlush(display)

def key(name, shift=False):
    guard()
    focused,revert = window_type(),c.c_int()
    x.XGetInputFocus(display,c.byref(focused),c.byref(revert))
    if focused.value!=window:
        x.XSetInputFocus(display,window,2,0)
        x.XFlush(display)
        time.sleep(0.03)
        x.XGetInputFocus(display,c.byref(focused),c.byref(revert))
    assert focused.value==window, 'Test window lost focus; input check stopped'
    code = x.XKeysymToKeycode(display, x.XStringToKeysym(name.encode()))
    assert code
    modifier = x.XKeysymToKeycode(display,x.XStringToKeysym(b'Shift_L'))
    if shift: xt.XTestFakeKeyEvent(display,modifier,1,0)
    xt.XTestFakeKeyEvent(display,code,1,0)
    xt.XTestFakeKeyEvent(display,code,0,0)
    if shift: xt.XTestFakeKeyEvent(display,modifier,0,0)
    x.XFlush(display)

def screenshot(name=None):
    guard()
    r, a, b, w, h, border, depth = window_type(), c.c_int(), c.c_int(), c.c_uint(), c.c_uint(), c.c_uint(), c.c_uint()
    assert x.XGetGeometry(display, window, c.byref(r), c.byref(a), c.byref(b), c.byref(w), c.byref(h), c.byref(border), c.byref(depth))
    ptr = x.XGetImage(display, window, 0, 0, w.value, h.value, c.c_ulong(-1), 2)
    assert ptr
    try:
        img = ptr.contents
        assert img.bits_per_pixel == 32 and img.byte_order == 0
        assert (img.red_mask,img.green_mask,img.blue_mask) == (0xff0000,0xff00,0xff)
        raw = c.string_at(img.data, img.bytes_per_line*img.height)
        bgra = b"".join(raw[row*img.bytes_per_line:row*img.bytes_per_line+img.width*4] for row in range(img.height))
        rgb = bytearray(img.width*img.height*3)
        rgb[0::3],rgb[1::3],rgb[2::3] = bgra[2::4],bgra[1::4],bgra[0::4]
        if name:
            def chunk(tag,data):
                return struct.pack('>I',len(data))+tag+data+struct.pack('>I',zlib.crc32(tag+data)&0xffffffff)
            rows = b"".join(b'\0'+rgb[row*img.width*3:(row+1)*img.width*3] for row in range(img.height))
            png = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',img.width,img.height,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(rows))+chunk(b'IEND',b'')
            (out/name).write_bytes(png)
        return hashlib.sha256(rgb).hexdigest(), img.width,img.height, rgb
    finally:
        x.XDestroyImage(ptr)

log_path = out/'desktop-check.log'
env = dict(os.environ)
env.pop('WAYLAND_DISPLAY',None)
env['VYSYN_REPRO_SESSION'] = 'desktop-check'
log = log_path.open('w')
proc = subprocess.Popen([str(root/'target/release/vysyn'),'--trace','--smoke-ms','15000',str(root/'artifacts/bench-images/gradient.png')],env=env,stdout=log,stderr=log)
display = x.XOpenDisplay(None)
if not display:
    proc.terminate(); raise SystemExit('An X11/Xwayland display is required')
try:
    window = None
    deadline = time.monotonic()+5
    while time.monotonic()<deadline and not window:
        listing = subprocess.check_output(['xprop','-root','_NET_CLIENT_LIST'],text=True)
        for ident in re.findall(r'0x[0-9a-fA-F]+',listing):
            prop = subprocess.run(['xprop','-id',ident,'_NET_WM_PID'],capture_output=True,text=True).stdout
            if re.search(r'=\s*'+str(proc.pid)+r'\s*$',prop):
                window = int(ident,16); break
        time.sleep(0.05)
    assert window, 'Viewer window not found'
    if env.get('HYPRLAND_INSTANCE_SIGNATURE'):
        clients = json.loads(subprocess.check_output(['hyprctl','-j','clients'],text=True))
        client = next(client for client in clients if client['pid']==proc.pid)
        assert b'VYSYN_REPRO_SESSION=desktop-check' in Path(f'/proc/{proc.pid}/environ').read_bytes().split(b'\0')
        answer = subprocess.check_output(['hyprctl','dispatch','hl.dsp.focus({window='+json.dumps('address:'+client['address'])+'})'],text=True).strip()
        assert answer=='ok', answer
        time.sleep(0.05)
    x.XSetInputFocus(display,window,2,0)
    x.XFlush(display)
    deadline = time.monotonic()+5
    while 'directory_files=' not in log_path.read_text() and time.monotonic()<deadline:
        assert proc.poll() is None, log_path.read_text()
        time.sleep(0.05)
    time.sleep(0.2)
    initial,w,h,rgb = screenshot('vysyn-default.png')
    center = tuple(rgb[((h//2)*w+w//2)*3:((h//2)*w+w//2)*3+3])
    assert max(center)>20, 'Image center is black'
    key('KP_Add'); time.sleep(0.15)
    assert screenshot()[0]!=initial, 'Keyboard zoom did not change rendering'
    key('0'); time.sleep(0.15)
    assert screenshot()[0]==initial, 'Fit did not restore initial rendering'
    event(6,0,w//2,h//2)
    event(4,4,w//2,h//2); event(5,4,w//2,h//2); time.sleep(0.15)
    zoomed = screenshot()[0]
    assert zoomed!=initial, 'Wheel zoom did not change rendering'
    event(4,1,w//2,h//2); event(6,0,w//2+70,h//2+30,state=256); event(5,1,w//2+70,h//2+30); time.sleep(0.15)
    assert screenshot()[0]!=zoomed, 'Dragging did not change rendering'
    key('0'); time.sleep(0.15)
    assert screenshot()[0]==initial, 'Fit did not clear panning'
    key('KP_Add'); time.sleep(0.15)
    event(4,1,w//2,h//2); event(6,0,w//2+70,h//2+30,state=256); event(5,1,w//2+70,h//2+30); time.sleep(0.15)
    changed = screenshot()[0]
    assert changed!=initial, 'Zoom and pan did not change rendering'
    event(4,1,w//2,h//2); event(5,1,w//2,h//2); time.sleep(0.1)
    assert screenshot()[0]==changed, 'Single click after dragging unexpectedly fitted the image'
    event(4,1,w//2,h//2); event(5,1,w//2,h//2); time.sleep(0.15)
    assert screenshot()[0]==initial, 'Double-click did not restore the same pixels as 0'
    time.sleep(0.5)
    key('Right'); time.sleep(0.3)
    assert screenshot()[0]!=initial, 'Next image did not change rendering'
    key('Left'); time.sleep(0.3)
    assert screenshot()[0]==initial, 'Previous image did not restore rendering'
    key('F11'); time.sleep(0.3)
    fullscreen = screenshot()[1:3]
    assert fullscreen!=(w,h), 'Fullscreen did not change window dimensions'
    key('F11'); time.sleep(0.3)
    key('Escape'); proc.wait(timeout=3)
    assert proc.returncode==0, log_path.read_text()
    report = {'viewport':[w,h],'center_rgb':center,'checks':['PNG pixels visible','keyboard zoom','fit','wheel zoom','pan','single click after drag','double-click matches 0','navigation right/left','fullscreen','Esc'],'log':str(log_path)}
    (out/'desktop-check.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))
finally:
    if proc.poll() is None:
        proc.terminate(); proc.wait(timeout=3)
    x.XCloseDisplay(display)
    log.close()

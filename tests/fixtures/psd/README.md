# PSD regression fixtures

The seven PSDs are unmodified test fixtures from
[psd-tools](https://github.com/psd-tools/psd-tools/tree/b58704c1c9c9b2459f961560b1e368dfb102b513/tests/psd_files),
pinned at `b58704c1c9c9b2459f961560b1e368dfb102b513`. Its MIT license is retained in `LICENSE`.

| Local file | Upstream path | SHA-256 |
|---|---|---|
| `16bit5x5.psd` | `tests/psd_files/16bit5x5.psd` | `c78689ea7b576f23bbd8b6b4f4993a365266c3ffc9aaa02cc20bef4a36a9d7a0` |
| `1layer.psd` | `tests/psd_files/1layer.psd` | `c2b457581d549f4bea2e5c34a04b68b2917f640bce0e958b6bca9c65be75174b` |
| `4x4_16bit_grayscale.psd` | `tests/psd_files/colormodes/4x4_16bit_grayscale.psd` | `5ee15880f77c1a0e2566864d8770a26efe3aa261245e360267973ebdd542a49c` |
| `4x4_8bit_grayscale.psd` | `tests/psd_files/colormodes/4x4_8bit_grayscale.psd` | `2811614db8536c363ffd4b9c97baf8965b58fda5c894ffeffe1067d1287edacf` |
| `4x4_8bit_rgba.psd` | `tests/psd_files/colormodes/4x4_8bit_rgba.psd` | `1f87ed0f6bace7587c0ab3fb48ffe22a7796a42e11743b6a10c42a4d2bd56d0f` |
| `semi-transparent-layers.psd` | `tests/psd_files/semi-transparent-layers.psd` | `2d190efddf648408ad0115852f8a029eec44de10bd29f6ff222aaa574cc31bc1` |
| `transparentbg-gimp.psd` | `tests/psd_files/transparentbg-gimp.psd` | `9c700d3e7ad0429ac3ea83a40bb02f364a3d4e7713e7d608e23a8e03014d3b9c` |

The adjacent PNGs are independent composite references generated locally with
ImageMagick 7.1.2-31 Q16-HDRI and its LCMS2 color engine on 2026-10-10:

```sh
magick 'input.psd[0]' -profile srgb.icc -strip PNG32:output.png
```

`srgb.icc` was the standard **sRGB IEC61966-2.1** ICC resource extracted from
`16bit5x5.psd` (resource 1039). ImageMagick selected
only the embedded composite, converted its source ICC to sRGB and exported
straight RGBA8. These are independent decoder/CMS results, not Photoshop exports.
The test applies Vysyn's linear alpha premultiplication to the PNGs before
comparing all pixels, with a one-code-value tolerance for CMS rounding.

The corpus includes Photoshop RGB/grayscale 8/16-bit profiles, layer adjustments,
extra saved alpha masks, and GIMP's transparent RLE composite. Synthetic tests
separately exercise ZIP/prediction, Photoshop matte removal and malformed input.

These are synthetic fixtures generated for Vysyn (no third-party image content).
`native-source.png` is a 64x48 red-to-blue gradient. `rotated.heic` contains its
HEIF container 90-degree rotation and `sample.avif` is an AV1-encoded equivalent.

Generated with ImageMagick and libheif 1.23.4:

```sh
magick -size 64x48 gradient:red-blue -depth 8 native-source.png
heif-enc -q 90 -p x265:pools=1 -p x265:frame-threads=1 \
  --colour_primaries 1 --transfer_characteristic 13 --rotate-cw 90 \
  -o rotated.heic native-source.png
heif-enc -A -q 90 --colour_primaries 1 --transfer_characteristic 13 \
  -o sample.avif native-source.png
```

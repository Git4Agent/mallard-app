# Claude media transport fixture

`clip.mp4` is a synthetic, silent, one-frame black 16 × 16 H.264 video.
It contains no user media. The E2E copies it under the regression filename
`Part5_The_Fall_of_Patroclus_zh.mp4` in each disposable non-Git project.

Generated with FFmpeg (no runtime FFmpeg dependency):

```sh
ffmpeg -f lavfi -i color=c=black:s=16x16:r=1 -frames:v 1 \
  -c:v libx264 -pix_fmt yuv420p -map_metadata -1 -movflags +faststart clip.mp4
```

Decoded successfully with `ffmpeg -i clip.mp4 -f null -`. The E2E checks
byte-for-byte transport and SHA-256; it does not claim browser playback.

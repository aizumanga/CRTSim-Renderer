# Test fixtures

- `four-frames.webp`: four lossless 4x2 frames of 100 ms, gray levels 0, 40, 80 and 120, from
  libwebp.

The videos are one second of FFmpeg's `testsrc2` at 64x48 and 10 frames per second, with a
keyframe every 5 frames, and one second of a 440 Hz tone where they have sound:

```sh
V="-f lavfi -i testsrc2=size=64x48:rate=10 -t 1"
A="-f lavfi -i sine=frequency=440:sample_rate=48000 -t 1"
ffmpeg $V $A -c:v libx264 -preset veryslow -bf 2 -pix_fmt yuv420p -g 5 -c:a aac -b:a 32k -ac 1 -shortest -movflags +faststart h264-aac.mp4
ffmpeg -display_rotation:v 90 -i h264-aac.mp4 -c copy -an rotated.mp4
ffmpeg $V $A -c:v libvpx-vp9 -b:v 50k -g 5 -c:a libopus -b:a 16k -ac 1 -shortest vp9-opus.webm
ffmpeg $V -c:v libvpx-vp9 -pix_fmt yuv420p10le -b:v 50k -g 5 vp9-10bit.webm
ffmpeg $V -c:v libvpx -b:v 50k -g 5 -live 1 -f webm live-vp8.webm
```

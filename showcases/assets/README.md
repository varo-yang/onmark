# Showcase assets

The checked-in media below is synthetic and exists only to exercise Onmark's
public media, audio, and subtitle paths without a third-party rights dependency.

`fractal.mp4` is a ten-second CFR H.264 source generated with:

```bash
ffmpeg -f lavfi \
  -i "mandelbrot=size=1280x720:rate=30:end_pts=300:end_scale=0.04" \
  -frames:v 300 -an -c:v libx264 -preset medium -crf 26 \
  -pix_fmt yuv420p -movflags +faststart \
  fractal.mp4
```

`pulse.wav` is a ten-second stereo PCM chord generated with:

```bash
ffmpeg -f lavfi \
  -i "aevalsrc=0.035*(sin(2*PI*110*t)+sin(2*PI*164.81*t)+sin(2*PI*220*t))*(0.65+0.35*sin(2*PI*0.25*t)):s=48000:d=10" \
  -af "afade=t=in:d=0.7,afade=t=out:st=9:d=1,pan=stereo|c0=c0|c1=c0" \
  -c:a pcm_s16le pulse.wav
```

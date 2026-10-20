# localflow: the dictation video

A ~50 s video of localflow's dictation, made with [Remotion](https://www.remotion.dev): a
message with fillers and a correction ("Thursday… wait, no, Friday") cleaned up and pasted
into a chat, then a rambled list turned into a numbered one. The cleaned text on screen is
what localflow made of the voiced clips (`localflow transcribe`, cleanup with Claude
Haiku 5.5); the pill is drawn like `crates/app/src/ui/overlay.rs`, its bars from the clips'
loudness.

```sh
npm i
npm run dev        # Remotion Studio
npm run render     # out/dictation.mp4 (1920×1080, 30 fps)
```

Voices are [Gradium](https://gradium.ai) TTS. To change a line, edit `scripts/lines.json`, then:

```sh
GRADIUM_API_KEY=gd_... python3 scripts/tts.py   # public/audio/<id>.wav, src/data/timings.json
python3 scripts/prepare.py                      # word timings and pill levels
```

If a dictation line changes, run its new clip through localflow (16 kHz WAV:
`ffmpeg -i public/audio/d_msg.wav -ar 16000 -ac 1 d_msg.16k.wav`, then
`localflow transcribe d_msg.16k.wav`) and put its output in `src/Video.tsx`.

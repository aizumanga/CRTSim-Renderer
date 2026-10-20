# The render service

`crtsim-service` renders pictures, animations and videos sent to it over HTTP through the CRT,
and sends them back: for a chat bot, a website's uploads or a batch of renders on a server with
no screen. It runs headless on any Vulkan, DX12 or Metal device, and its container image runs
on Mesa's software Vulkan, so it needs no GPU at all. One renderer draws one job at a time; a
queue of bounded length holds the rest, and every request, render and result has a limit.

## Running it

From the repository, on a machine with a GPU driver or Mesa's software Vulkan
(`mesa-vulkan-drivers` on Debian and Ubuntu):

```sh
cargo run --release -p crtsim-service -- --bind 127.0.0.1:8080
```

Or as a container, which carries software Vulkan and FFmpeg:

```sh
docker build -f packaging/service/Dockerfile -t crtsim-service .
docker run --rm -p 127.0.0.1:8080:8080 -e CRTSIM_SERVICE_TOKEN="$(openssl rand -hex 24)" crtsim-service
```

It says what it renders on, and whether it can render animations and videos, which needs
FFmpeg as the desktop's video does:

```sh
curl localhost:8080/health
```

## Rendering

Send the picture as the request's body, and the options in the query. The answer is the
rendered file:

```sh
curl --data-binary @screenshot.png -o crt.png \
  'localhost:8080/render?preset=NTSC%20240p&size=1080p'
curl --data-binary @clip.gif -o crt.gif 'localhost:8080/render?max_side=480&fps=24&seconds=4'
curl --data-binary @clip.mp4 -o crt.mp4 'localhost:8080/render?preset=NTSC%20480i&quality=high'
```

Or send JSON, with the file in base64 as `input` and a look's whole settings, the JSON the app
saves as a preset, as `settings`:

```json
{ "input": "iVBORw0KGgo…", "settings": { "signal": "240p", "bloom": 0.4 }, "color_space": "display-p3" }
```

`POST /render` answers with the result when it is ready within the service's wait (60 seconds by
default). A job that takes longer is answered `202 Accepted`, with the job to ask after.
`POST /jobs` queues a job and answers at once, for work that is known to take a while:

| Request | Answer |
| --- | --- |
| `GET /` | the endpoints and options, as JSON |
| `GET /health` | whether it runs, on what adapter, whether it renders animations and videos, and how many jobs wait |
| `GET /presets` | the included looks, for `preset` |
| `POST /render` | the result, or `202` and the job if it takes longer than the wait |
| `POST /jobs` | `202` and the job: `{"id", "status", "ahead"}`, with `Location: /jobs/{id}` |
| `GET /jobs/{id}` | its `status` (`queued`, `running`, `done`, `failed` or `cancelled`), `progress` from 0 to 1, `stage`, and an `error` if it failed |
| `GET /jobs/{id}/result` | the file once done; `409` before, and the job's error after a failure |
| `DELETE /jobs/{id}` | stops it, or forgets its result |

| Option | What it sets |
| --- | --- |
| `preset` | an included look by name, as `GET /presets` lists them |
| `settings` | a look's whole settings, as a preset's JSON (in JSON, an object; in the query, the JSON as text) |
| `size` | the CRT canvas: `720p`, `1080p`, `1440p`, `4k`, `reference`, `match-input` or `WIDTHxHEIGHT` |
| `signal` | `original`, `auto`, `native`, `240p`, `288p`, `360p`, `480p`, `576p` or `WIDTHxHEIGHT` |
| `format` | `png`, `gif`, `webp`, `mp4`, `mkv` or `webm`; by default the input's own kind: a still as a PNG, an animated GIF or WebP as itself, a video as MP4. A moving input as `png` is an animated PNG |
| `color_space` | a still's: `srgb`, `display-p3`, `adobe-rgb`, `rec2020` or `prophoto-rgb` |
| `color_profile` | a still's colour space as an RGB ICC profile, in base64 (JSON only) |
| `ignore_profile` | `true` reads the input's numbers as sRGB, whatever colour profile it carries |
| `timing` | `stable`, `ntsc60` (videos only) or `off` |
| `fps`, `seconds`, `start` | an animation's frame rate, and the stretch of the input it renders |
| `max_side` | an animation's or a video's canvas, its longest side in pixels |
| `quality` | a video's: `draft`, `balanced`, `high`, `archival` or `lossless` (MKV) |
| `lossless` | `true` for a lossless WebP animation |

An option the service does not know, or a value it cannot use, is refused with `400` and a
sentence saying why, as `{"error": "…"}`. Stills take PNG, JPEG, WebP, BMP and GIF; videos MP4,
MOV, MKV, WebM and AVI.

## Limits

Each is a command-line option; the defaults suit a small server:

| Option | Default | Refused with |
| --- | --- | --- |
| `--max-upload-mb` | 64 | `413` before the upload is read |
| `--max-input-megapixels` | 16.8, a 4096×4096 still | `422` |
| `--max-output-megapixels` | 8.3, 4K | `422` |
| `--max-seconds` | 30, of a video or of an animation's stretch | `422` |
| `--max-moving-side` | 1280 | `422` |
| `--max-queue` | 16 jobs waiting | `503`, with `Retry-After` |
| `--max-connections` | 64 at once | `503`, with `Retry-After` |
| `--still-timeout` | 120 seconds | the job stops; its result is `504` |
| `--moving-timeout` | 900 seconds | the job stops; its result is `504` |
| `--wait` | 60 seconds | `POST /render` answers `202` with the job instead |
| `--keep` | 600 seconds | a result is forgotten this long after it finished |

A request has 120 seconds to arrive in full, and a response as long to be sent. Results and
uploads are kept in a temporary folder that is removed when the service stops.

## Exposing it

By default it listens on this computer alone. To reach it from elsewhere:

- **Set a token**, at least 16 characters, in `CRTSIM_SERVICE_TOKEN` or a file named by
  `--token-file`. Every request but `GET /health` must then send `Authorization: Bearer <token>`;
  one without it is refused before its upload is read. Without a token, the service warns when
  it listens beyond this computer.
- **Put it behind a reverse proxy** that speaks HTTPS, such as Caddy or nginx. The service speaks
  plain HTTP/1.1, one request a connection, and leaves TLS and HTTP/2 to the proxy:

  ```
  crt.example.com {
      reverse_proxy 127.0.0.1:8080
  }
  ```
- **Jobs are known by 128 random bits**, so a result can be fetched only by whoever queued it,
  and is forgotten after `--keep`.
- **FFmpeg reads what people upload.** Animations and videos are probed and decoded by FFmpeg,
  as on the desktop; the container runs it as an unprivileged user, with nothing of the host's
  but the published port. A service that renders stills only can run without FFmpeg, and then
  refuses animations and videos with `501`.


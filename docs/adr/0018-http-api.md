# ADR-0018: HTTP API design

- Status: Accepted
- Date: 2026-10-12

## Context

ADR-0007 fixed the shape: stateless, one image per request, raw bytes in, compressed bytes out. This ADR pins down the details the web UI and any script will depend on: paths, parameters, headers, error bodies and limits.

The only client in v1 is our own page, embedded in the same binary (ADR-0013). Scripts may also call it with `curl`.

## Decision

### Routes

| Method and path | Purpose |
|---|---|
| `POST /api/compress` | Compress one image |
| `GET /api/info` | Version, supported formats, limits and defaults |
| `GET /api/health` | `200`, body `ok` |
| `GET /` and `GET /{file}` | The embedded web UI from `web/` |

Paths have no version prefix. The UI and the API ship in the same binary, so they can't drift apart. If the web version goes public (ADR-0014), that ADR adds `/api/v1/`.

### `POST /api/compress`

Request:

- Query parameters `quality`, `effort` and `metadata`, all optional, with the grammar from ADR-0017.
- The body is the raw file. `Content-Type` is ignored, because the core detects the format from magic bytes.
- A `Content-Length` above the limit gets `413` before any body is read. Chunked bodies are cut off at the same limit.

Success, `200`:

```http
HTTP/1.1 200 OK
Content-Type: image/png
Content-Length: 48211
X-Original-Size: 182344
X-Compressed-Size: 48211
X-Kept-Original: false
Server-Timing: compress;dur=84.2
Cache-Control: no-store
```

When the size guard keeps the original, the status is still `200`, the body is the original bytes and `X-Kept-Original` is `true`.

Errors return JSON with a code from ADR-0017 or one of the HTTP-only codes below:

```json
{ "error": "unsupported_format", "message": "AVIF is not supported yet" }
```

| Status | `error` | When |
|---|---|---|
| `400` | `empty_body` | Body has zero bytes |
| `400` | `invalid_option` | A query parameter doesn't parse. `message` names it. |
| `413` | `body_too_large` | Body above `--max-body` (default 25 MB) |
| `413` | `too_many_pixels` | Header dimensions above `--max-pixels` (default 50 MP) |
| `415` | `unsupported_format` | Unknown or not compiled in |
| `415` | `animated_image` | APNG, animated WebP, GIF |
| `422` | `decode_failed` | Bytes look like a known format but don't decode |
| `429` | `busy` | Every job slot is taken. Sent with `Retry-After: 1`. |
| `500` | `encode_failed` | Encoder error, or a panic caught per ADR-0017 |
| `504` | `timeout` | The job ran past `--timeout` (default 30 s) |

### Concurrency and timeouts

- A semaphore with one permit per job slot (`--jobs`, default: core count). The handler calls `try_acquire`. No permit means an immediate `429`, not a queue.
- The permit moves into the `spawn_blocking` closure and is released only when the work actually finishes. A blocking task can't be cancelled, so after a `504` the job keeps its slot until it's done. Timeouts can't be used to start more jobs than there are cores.

### `GET /api/info`

The page reads this on load to reject oversized files before uploading them and to show the version.

```json
{
  "version": "0.1.0",
  "formats": ["png", "jpeg"],
  "limits": { "max_body_bytes": 26214400, "max_pixels": 50000000, "timeout_ms": 30000, "jobs": 12 },
  "defaults": { "quality": "auto", "effort": "default", "metadata": "strip-keep-icc" }
}
```

`formats` lists what this build can compress, so it grows to include `webp` in Phase 2 without a UI change.

### Static files

- Served from the files embedded with rust-embed. Unknown paths get `404`.
- `ETag` from rust-embed's file hash, with `Cache-Control: no-cache`, so the browser revalidates and a new binary never serves stale JS.

### No CORS

The server sends no CORS headers. Only the page served by squish itself can read responses. Another website open in the same browser can't use a local squish to read compressed images back.

## Consequences

- `curl --data-binary @photo.png localhost:8080/api/compress -o small.png` works with no client library.
- The UI shows sizes from headers without parsing anything, and it handles errors by switching on `error`.
- Because a `429` comes back at once, the browser's retry with backoff (FR-19) does the queuing. The server holds no queue and no memory for waiting jobs.
- A burst of slow images can make the server answer `429` for up to the timeout. That's the intended back-pressure.

## Alternatives considered

- **Multipart uploads.** Lets one request carry a filename and several files, but needs a parser, and we can't reject a bad file after reading only its first bytes.
- **JSON response with base64 image.** One parse step for metadata, but 33% bigger responses and an extra copy on both ends.
- **A server-side queue instead of `429`.** Hides load from the client and needs a queue length limit anyway.
- **`/api/v1/` now.** Costs nothing today, but solves a compatibility problem we can't have while the UI ships inside the binary.

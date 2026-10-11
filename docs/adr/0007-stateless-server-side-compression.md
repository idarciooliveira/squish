# ADR-0007: Compress on the server, keep the server stateless

- Status: Accepted
- Date: 2026-10-11

## Context

The web version can compress images on the server or in the browser through WASM. Server-side compression runs native code and reuses the CLI's core. Browser-side keeps images private and makes hosting free, but the C codecs need Emscripten and phones are slow.

If the server does the work, it can either answer each upload with the compressed file directly, or store files under a job ID and let the browser poll and download later.

## Decision

1. Compress on the server for v1.
2. The server never stores a file. The browser sends one image per request to `POST /api/compress` with the raw bytes as the body, and the response is the compressed image with size headers.
3. The browser runs up to 4 uploads at a time and retries on `429`.
4. The server limits concurrent jobs to the number of CPU cores with a semaphore, runs the CPU work in `spawn_blocking`, and returns `429` when every slot is taken.
5. In-browser compression stays a Phase 4 experiment.

## Consequences

- No temp files, no cleanup, no sessions, nothing shared between users. The server can restart at any time.
- Raw bodies instead of multipart let us reject a bad upload after reading its first bytes.
- Very large batches cost one request per file, which is fine at our scale.
- Results only live in the browser tab. Closing it loses them, which matches how TinyPNG behaves.

## Alternatives considered

- **Job IDs with server storage.** Needed for resumable uploads or a public API. Adds storage, expiry and cleanup. Not needed yet.
- **Browser-only (WASM).** See ADR-0005 for the JPEG problem. Revisit after v1.

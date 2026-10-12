# ADR-0021: Optional Tinify-compatible API

- Status: Deferred
- Date: 2026-10-12

## Context

We want code that already calls TinyPNG to work against squish with as little change as possible. We checked what "the TinyPNG interface" actually is on 2026-10-12.

### There is no official CLI

tinify.com lists client libraries for Ruby, PHP, Node.js, Python, Java and .NET, a WordPress plugin, and third-party plugins. It doesn't ship a CLI. The best-known community one, `tinypng-cli` on npm, overwrites files in place and takes `-k KEY`, `-r`, `--width`/`--height`, `-m/--max`, `--cache`, `--force` and `--dry-run`. It isn't a standard anyone depends on, so ADR-0019 stands as is.

### The HTTP API is a two-step protocol

Base URL `https://api.tinify.com`, HTTP Basic auth with user `api` and the key as password.

1. `POST /shrink` with the raw image as the body, or JSON `{"source": {"url": "..."}}`. The response is `201 Created` with a `Location: https://api.tinify.com/output/<id>` header, a `Compression-Count` header, and JSON:
   ```json
   { "input":  { "size": 207565, "type": "image/jpeg" },
     "output": { "size": 63669, "type": "image/jpeg", "width": 1030, "height": 1030,
                 "ratio": 0.3067, "url": "https://api.tinify.com/output/<id>" } }
   ```
2. `GET /output/<id>` returns the image bytes, with `Image-Width` and `Image-Height` headers.
3. `POST /output/<id>` with a JSON body applies `preserve` (`copyright`, `creation`, `location`), `resize` (`scale`, `fit`, `cover`, `thumb`), `convert`, `transform.background` or `store` (S3 and GCS), and returns the new image.
4. Errors are JSON `{"error": "<Name>", "message": "<text>"}`. The clients map `401` and `429` to `AccountError`, other 4xx to `ClientError` and 5xx to `ServerError`. They retry once on 5xx.

The clients only read the `Location` header from step 1, not the JSON body. Their `validate()` call sends an empty `POST /shrink` and treats any 4xx other than `401` as a valid key.

### Official clients can't be repointed by configuration

| Client | Endpoint | Trust |
|---|---|---|
| Python | class attribute `Client.API_ENDPOINT` | its own bundled `cacert.pem` |
| Node.js | `private static API_ENDPOINT`, calls `https.request` only | its own CA bundle |
| PHP | `const API_ENDPOINT` | its own `cacert.pem` via `CURLOPT_CAINFO` |
| Ruby | frozen constant `API_ENDPOINT` | its own `cacert.pem` |
| Java | `public static final String API_ENDPOINT` | |
| WordPress plugin | the PHP client, or `const SHRINK_URL` for its fallback driver | |

None of them has a setting for the base URL. Python, PHP and Java treat any URL that doesn't start with `https` as a path and prefix their own endpoint to it. Node only speaks HTTPS. Because the clients pin their own CA bundles, a self-signed certificate or a local CA won't be trusted either.

So a true drop-in replacement for an unmodified SDK isn't possible. What we can offer is wire compatibility:

- Code that calls the HTTP API itself (curl, scripts, plugins with a configurable host) only changes the base URL.
- SDK users override one constant at startup, for example `tinify.Client.API_ENDPOINT = "http://127.0.0.1:8080"` in Python or Ruby. Node users also need HTTPS with a publicly trusted certificate. PHP and Java constants can't be overridden at runtime, so those users need a patched client or an HTTP integration.

## Decision

Add the Tinify routes as an opt-in mode, next to our own API, not instead of it.

`squish serve --tinify` mounts:

| Route | Behavior |
|---|---|
| `POST /shrink` | Raw body only. Compresses with default options, stores the result, answers `201` with `Location`, `Compression-Count` and the JSON body above. |
| `GET /output/{id}` | The stored bytes, with `Content-Type`, `Image-Width` and `Image-Height` |
| `POST /output/{id}` | `preserve` only. Re-runs compression from the stored input with `keep-all` metadata when anything is listed (`location` included), since we don't copy single tags. `resize`, `convert`, `transform` and `store` return `400` with a clear message. |

Details:

- **Storage.** An in-memory cache keyed by a random 16-character id, holding input and output bytes. Entries expire after 10 minutes, and the cache evicts the oldest entries above 256 MB (`--tinify-cache-mb`). This mode isn't stateless, which ADR-0007 only allows here. Nothing touches the disk, and a restart drops everything.
- **Auth.** Any Basic auth is accepted by default, so existing keys keep working. `--tinify-key KEY` (env `SQUISH_TINIFY_KEY`) makes squish reject other keys with `401 Unauthorized`.
- **`Location`.** A relative path such as `/output/abc123`, which every client resolves against its endpoint. With `--public-url https://img.example.com`, it's an absolute URL instead.
- **`source.url` uploads.** Rejected with `400`. Fetching arbitrary URLs from the server is an SSRF risk, and local-first squish has no need for it. A later ADR can add it behind an allow-list.
- **`Compression-Count`.** A counter for this process, reset on restart. Clients only parse it.
- **Error names.** We match Tinify's `error` strings. Before implementing, we capture real responses with a free API key (empty body, bad image, bad key, unsupported option) into `crates/server/tests/fixtures/tinify/`, and the route tests compare against them.
- **Phase.** Not scheduled. See "When to revisit" below. Our web UI keeps using `/api/compress` either way.

## When to revisit

Deferred on 2026-10-12. The integrations that would gain most from a drop-in (WordPress, PHP and Java backends) can't be repointed at all, and nobody has asked for it yet. The native API is a thin layer over the core, so adding these routes later costs the same as adding them now.

Pick this ADR up again, and set it to Accepted, when either of these happens:

1. A concrete integration that calls Tinify needs to run against squish.
2. We decide to take the web version public (ADR-0014).

Until then, keep `/shrink` and `/output/` free. No file in `web/` and no native route may use those paths.

## Consequences

- Scripts and HTTP integrations written for TinyPNG work after a base URL change.
- Python and Ruby SDK users can switch with one line. Node, PHP and Java SDK users have a harder path, and the docs must say so plainly instead of promising a drop-in.
- Two API styles to maintain. The compat routes are thin: they call the same core and reuse ADR-0017's options and errors.
- The cache costs memory while the mode is on. It's bounded by the cap and the TTL.
- The PRD non-goal "a public API with keys" still holds. The optional key is one shared secret, not accounts.

## Alternatives considered

- **Replace our native API with the Tinify shape.** Every image would cost two round trips and need server storage, and our own UI gains nothing from that.
- **Intercept `api.tinify.com` with DNS or `/etc/hosts`.** The SDKs pin their CA bundles, so this needs a certificate for `api.tinify.com` that those bundles trust. We can't get one.
- **Stateless compat by putting the image in the `Location` URL.** URLs can't hold megabytes.
- **Implement `resize` and `convert` too.** Those break the v1 non-goals (no editing, no format conversion). Clients that need them get a clear `400` instead of a silent wrong result.
- **Match the community `tinypng-cli` flags.** Its in-place default and cache file make it a different tool. Few scripts depend on it.

# Vendored scripts

| File | Source | Version | License |
|---|---|---|---|
| `client-zip.js` | `index.js` from the [client-zip](https://github.com/Touffy/client-zip) npm package, unmodified | 2.5.1 | MIT, see `client-zip.LICENSE.txt` |

client-zip only writes STORE entries, which is what "Download all" needs (ADR-0008).
To update, download `https://unpkg.com/client-zip@<version>/index.js` and check it against the
`integrity` field that `https://unpkg.com/client-zip@<version>/?meta` lists. Version 2.5.1 is
`sha256-KMlXxT2OUpCVHUtFzP+KuMnDchnaCLnVN7vdWVp1cG0=`.

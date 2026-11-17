# Test-only TLS certificates

For the MLLP listener's TLS tests (HUB-3, NFR-31). **Never use them anywhere else.** The private keys are
public, here, on purpose.

- `ca.pem`: a test CA. Its key was not kept.
- `server.pem`, `server.key`: `localhost` and `127.0.0.1`, signed by `ca.pem`.
- `client.pem`, `client.key`: a client certificate signed by `ca.pem` (accepted when the listener asks for one).
- `other-ca.pem`, `stranger.pem`, `stranger.key`: a client certificate signed by another CA (refused).

Made 2026-11-17 with OpenSSL (EC P-256, valid 100 years):
`openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256`, then `openssl req` and `openssl x509 -req`.

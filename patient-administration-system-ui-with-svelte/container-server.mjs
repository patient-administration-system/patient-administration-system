// The container's entry point: adapter-node's handler behind a plain HTTP listener.
//
// adapter-node (6, with SvelteKit 3) works out the page's origin from the request headers and
// assumes `https` unless the protocol header says otherwise. Over plain HTTP (a local stack, the
// demonstration) a form post's `Origin: http://…` then never matches `https://…` and SvelteKit refuses
// it as cross-site (403). So: read the protocol from `x-forwarded-proto` (set PROTOCOL_HEADER), and when
// no proxy set it, say `http`. Behind a TLS-terminating proxy that sets the header, nothing changes.
import http from "node:http";
import { handler } from "./build/handler.js";

const host = process.env.HOST ?? "0.0.0.0";
const port = Number(process.env.PORT ?? 3000);
const fallback = (process.env.UI_PROTOCOL ?? "http").toLowerCase();

http
  .createServer((req, res) => {
    if (!req.headers["x-forwarded-proto"]) req.headers["x-forwarded-proto"] = fallback;
    handler(req, res, () => {
      res.statusCode = 404;
      res.end();
    });
  })
  .listen(port, host, () => console.log(`Listening on http://${host}:${port}`));

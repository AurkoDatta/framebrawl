// Static file server for the harness page, and for the static build of
// the game under /static/, the way a project page on a static host serves
// it from a sub-path. Browsers refuse to compile wasm that is not served
// as application/wasm.
import { createReadStream, existsSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL(".", import.meta.url));
const staticBuild = fileURLToPath(new URL("../../web/dist-static/", import.meta.url));
const types = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".wasm": "application/wasm",
  ".css": "text/css",
  ".woff2": "font/woff2",
  ".woff": "font/woff",
};

createServer((req, res) => {
  const path = normalize(decodeURIComponent(new URL(req.url, "http://x").pathname));
  const inStatic = path.startsWith("/static/");
  const base = inStatic ? staticBuild : root;
  const relative = inStatic ? path.slice("/static/".length) || "index.html" : path;
  const file = join(base, path === "/" ? "harness/index.html" : relative);
  if (!file.startsWith(base) || !existsSync(file) || !statSync(file).isFile()) {
    res.writeHead(404).end();
    return;
  }
  res.writeHead(200, { "content-type": types[extname(file)] ?? "application/octet-stream" });
  createReadStream(file).pipe(res);
}).listen(4173, "127.0.0.1");

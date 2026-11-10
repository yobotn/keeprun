// Test app for Keeprun: an HTTP server that also spawns a long running child process,
// so stopping it must end a process tree (cmd > npm > node > node).
const http = require("http");
const { spawn } = require("child_process");

const port = Number(process.env.PORT || 3999);
const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "ignore" });
console.log(`child pid ${child.pid}`);

http
  .createServer((req, res) => res.end(`hello from tree-app (${process.env.GREETING || "no greeting"})\n`))
  .listen(port, () => console.log(`listening on http://localhost:${port}`));

setInterval(() => console.log(`tick ${new Date().toISOString()}`), 2000);

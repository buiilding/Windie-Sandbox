const { spawn } = require("node:child_process");
const path = require("node:path");

const server = path.join(
  __dirname,
  "node_modules",
  "chrome-devtools-mcp",
  "build",
  "src",
  "bin",
  "chrome-devtools-mcp.js",
);

const mode = process.env.WINDIE_CHROME_CONNECTION_MODE || "managed";
const args = [server];

if (mode === "existing") {
  args.push("--auto-connect");
} else {
  args.push(
    "--user-data-dir",
    process.env.WINDIE_CHROME_PROFILE_DIR || path.join(__dirname, "profile"),
  );
}

args.push("--no-usage-statistics");

const child = spawn(process.execPath, args, {
  env: process.env,
  stdio: "inherit",
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code === null ? 1 : code);
  }
});

for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(signal, () => child.kill(signal));
}

"use strict";

const childProcess = require("node:child_process");
const dns = require("node:dns");
const http = require("node:http");
const https = require("node:https");
const { syncBuiltinESMExports } = require("node:module");
const net = require("node:net");
const tls = require("node:tls");

const blocked = [];

function deny(operation) {
  return function blockedOperation() {
    blocked.push(operation);
    const error = new Error(`blocked:${operation}`);
    error.code = "POC_EXTERNAL_OPERATION_BLOCKED";
    throw error;
  };
}

globalThis.__MALLARD_QODER_FORK_GUARD__ = Object.freeze({
  blocked,
});
globalThis.fetch = deny("fetch");

for (const name of ["request", "get"]) {
  http[name] = deny(`http.${name}`);
  https[name] = deny(`https.${name}`);
}

for (const name of ["connect", "createConnection"]) {
  net[name] = deny(`net.${name}`);
}
tls.connect = deny("tls.connect");

for (const name of ["lookup", "resolve", "resolve4", "resolve6"]) {
  dns[name] = deny(`dns.${name}`);
}

for (const name of [
  "exec",
  "execFile",
  "execFileSync",
  "execSync",
  "fork",
  "spawn",
  "spawnSync",
]) {
  childProcess[name] = deny(`child_process.${name}`);
}

syncBuiltinESMExports();

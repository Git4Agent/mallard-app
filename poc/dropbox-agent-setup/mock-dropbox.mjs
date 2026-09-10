import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { authorizeUser, SETUP_SCOPES, STORAGE_SCOPES } from "./setup-poc.mjs";

// All values are synthetic. This server is reachable only on loopback.
export class MockDropbox {
  static async start(faults = {}) {
    const mock = new MockDropbox(faults);
    mock.server = createServer((request, response) => {
      mock.handle(request, response).catch(() => {
        if (!response.destroyed) response.writeHead(500).end();
      });
    });
    await new Promise((resolve, reject) => {
      mock.server.once("error", reject);
      mock.server.listen(0, "127.0.0.1", resolve);
    });
    mock.origin = `http://127.0.0.1:${mock.server.address().port}`;
    return mock;
  }

  constructor(faults) {
    this.faults = faults;
    this.calls = [];
    this.folder = null;
    this.shared = false;
    this.refreshed = false;
    this.tokenExchanged = false;
    this.polls = 0;
    this.metadataReads = 0;
    this.accountId = "dbid:mock-account";
    this.namespaceId = "mock-shared-namespace";
    this.appKey = "mockAppKey";
    this.code = "mock-auth-code-canary";
    this.setupToken = "mock-setup-access-canary";
    this.accessToken = "mock-refreshed-access-canary";
    this.refreshToken = "mock-refresh-token-canary";
  }

  options(overrides = {}) {
    return {
      appKey: this.appKey,
      fetchImpl: this.fetch.bind(this),
      authorize: () => authorizeUser({ appKey: this.appKey, port: 0, timeoutMs: 1000,
        openBrowser: this.browser.bind(this) }),
      sleep: async () => {},
      maxPolls: 3,
      ...overrides,
    };
  }

  async fetch(url, options) {
    const target = new URL(url);
    if (target.origin !== "https://api.dropboxapi.com") throw new Error("unexpected_mock_origin");
    return fetch(`${this.origin}${target.pathname}${target.search}`, options);
  }

  async browser(url) {
    const target = new URL(url);
    if (target.origin !== "https://www.dropbox.com" || target.pathname !== "/oauth2/authorize") {
      throw new Error("unexpected_mock_authorization_origin");
    }
    // Simulates user consent only against this test server. No real browser.
    const response = await fetch(`${this.origin}${target.pathname}${target.search}`, { redirect: "manual" });
    await response.body?.cancel();
    const callback = new URL(response.headers.get("location"));
    if (this.faults.wrongStateFirst) {
      const wrong = new URL(callback);
      wrong.searchParams.set("state", "attacker-state");
      const invalid = await fetch(wrong);
      this.invalidStateStatus = invalid.status;
      await invalid.body?.cancel();
    }
    const completed = await fetch(callback);
    this.callbackText = await completed.text();
  }

  sharedMetadata() {
    return { ".tag": "complete", shared_folder_id: this.faults.homeNamespace ? "mock-home" : this.namespaceId,
      access_type: { ".tag": this.faults.viewer ? "viewer" : "owner" },
      path_lower: this.folder.path_lower,
      ...(this.faults.changedFolderId ? {} : { folder_id: this.folder.id }),
      name: this.folder.name, is_inside_team_folder: false, is_team_folder: false,
      policy: { acl_update_policy: { ".tag": "owner" }, member_policy: { ".tag": "anyone" } },
      preview_url: "https://www.dropbox.com/mock", access_inheritance: { ".tag": "inherit" } };
  }

  async handle(request, response) {
    const url = new URL(request.url, this.origin);
    const parts = [];
    let size = 0;
    for await (const part of request) {
      size += part.length;
      if (size > 65536) { response.writeHead(413).end(); return; }
      parts.push(part);
    }
    const raw = Buffer.concat(parts).toString("utf8");
    const body = url.pathname === "/oauth2/token" ? Object.fromEntries(new URLSearchParams(raw)) : raw ? JSON.parse(raw) : null;
    const root = request.headers["dropbox-api-path-root"] ? JSON.parse(request.headers["dropbox-api-path-root"]) : null;
    this.calls.push({ path: url.pathname, body, root, authorization: request.headers.authorization });
    const json = (status, value) => response.writeHead(status, { "Content-Type": "application/json" }).end(JSON.stringify(value));
    const rejected = () => json(409, { error_summary: "mock-private-response-canary" });
    if (this.faults.status && url.pathname === this.faults.statusRoute) {
      json(this.faults.status, { error_summary: "mock-private-response-canary" }); return;
    }
    if (this.faults.redirect && url.pathname === "/oauth2/token") {
      response.writeHead(302, { Location: `${this.origin}/must-not-follow` }).end(); return;
    }
    if (url.pathname === "/oauth2/authorize") {
      const params = url.searchParams;
      if (params.get("client_id") !== this.appKey || params.get("response_type") !== "code"
        || params.get("code_challenge_method") !== "S256" || params.get("token_access_type") !== "offline"
        || params.get("scope") !== SETUP_SCOPES.join(" ")) { rejected(); return; }
      this.challenge = params.get("code_challenge");
      this.redirectUri = params.get("redirect_uri");
      const callback = new URL(this.redirectUri);
      callback.searchParams.set("state", params.get("state"));
      callback.searchParams.set(this.faults.denied ? "error" : "code", this.faults.denied ? "access_denied" : this.code);
      response.writeHead(302, { Location: callback.toString() }).end(); return;
    }
    if (url.pathname === "/oauth2/token") {
      if (body.client_id !== this.appKey || body.client_secret || url.search) { rejected(); return; }
      if (body.grant_type === "authorization_code") {
        const challenge = createHash("sha256").update(body.code_verifier).digest("base64url");
        if (this.tokenExchanged || body.code !== this.code || challenge !== this.challenge
          || this.faults.wrongVerifier || body.redirect_uri !== this.redirectUri) { rejected(); return; }
        this.tokenExchanged = true;
        const grant = { access_token: this.setupToken, refresh_token: this.refreshToken, token_type: "bearer",
          expires_in: 14400, account_id: this.accountId, scope: SETUP_SCOPES.join(" ") };
        if (this.faults.missingRefresh) delete grant.refresh_token;
        if (this.faults.missingScope) grant.scope = STORAGE_SCOPES.join(" ");
        if (this.faults.wrongAccount) grant.account_id = "dbid:another-account";
        json(200, grant); return;
      }
      if (body.grant_type === "refresh_token") {
        if (body.refresh_token !== this.refreshToken || this.faults.revokedRefresh
          || body.scope !== STORAGE_SCOPES.join(" ")) { rejected(); return; }
        this.refreshed = true;
        const renewed = { access_token: this.accessToken, token_type: "bearer", expires_in: 14400,
          scope: STORAGE_SCOPES.join(" ") };
        if (this.faults.omitRefreshScope) delete renewed.scope;
        json(200, renewed); return;
      }
      rejected(); return;
    }
    const isRenewed = request.headers.authorization === `Bearer ${this.accessToken}` && this.refreshed;
    const isSetup = request.headers.authorization === `Bearer ${this.setupToken}` && this.tokenExchanged && !this.refreshed;
    if (!isRenewed && !isSetup) { json(401, {}); return; }
    if (request.method !== "POST" || request.headers["dropbox-api-select-user"] || request.headers["dropbox-api-select-admin"]) {
      rejected(); return;
    }
    switch (url.pathname) {
      case "/2/users/get_current_account":
        json(200, { account_id: this.faults.switchedRefreshAccount && isRenewed ? "dbid:switched" : this.accountId,
          root_info: { ".tag": this.faults.team ? "team" : "user", home_namespace_id: "mock-home",
            root_namespace_id: this.faults.team ? "mock-team-root" : "mock-home" } }); return;
      case "/2/files/create_folder_v2":
        if (!isSetup || root?.[".tag"] !== "home" || body.autorename !== false || this.folder || this.faults.collision) {
          rejected(); return;
        }
        this.folder = { id: "id:mock-folder", path_lower: body.path.toLowerCase(), name: body.path.slice(1) };
        if (this.faults.lostCreateResponse) { request.socket.destroy(); return; }
        json(200, { metadata: this.folder }); return;
      case "/2/sharing/share_folder":
        if (!isSetup || body.path !== this.folder?.id || root?.[".tag"] !== "home") { rejected(); return; }
        this.shared = true;
        if (this.faults.lostShareResponse) { request.socket.destroy(); return; }
        json(200, this.faults.async ? { ".tag": "async_job_id", async_job_id: "mock-job" } : this.sharedMetadata()); return;
      case "/2/sharing/check_share_job_status":
        if (!this.shared || body.async_job_id !== "mock-job") { rejected(); return; }
        this.polls += 1;
        json(200, this.faults.neverComplete || this.polls === 1 ? { ".tag": "in_progress" }
          : this.faults.asyncFailed ? { ".tag": "failed", failed: { ".tag": "no_permission" } } : this.sharedMetadata()); return;
      case "/2/files/get_metadata":
        if ((!isRenewed && !isSetup) || ![this.folder?.id, this.folder?.path_lower].includes(body.path.toLowerCase())
          || root?.[".tag"] !== "home") { rejected(); return; }
        this.metadataReads += 1;
        json(200, { ...this.folder, id: this.faults.changedFolderId ? "id:changed-folder" : this.folder.id,
          path_lower: this.faults.wrongFolderPath ? "/another-folder" : this.folder.path_lower,
          ".tag": "folder", ...(this.metadataReads <= (this.faults.metadataDelay ?? 0) ? {} : { sharing_info: {
            shared_folder_id: this.faults.wrongNamespace ? "another-namespace" : this.namespaceId } }) }); return;
      case "/2/files/list_folder":
        if (!isRenewed || root?.namespace_id !== this.namespaceId || root?.[".tag"] !== "namespace_id" || body.path !== "") {
          rejected(); return;
        }
        json(200, { entries: [], cursor: "mock-list-cursor", has_more: false }); return;
      default: rejected();
    }
  }

  async close() {
    this.server.closeAllConnections();
    await new Promise((resolve) => this.server.close(resolve));
  }
}

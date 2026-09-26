# Feishu: Mallard's app, and installing it in your organization

Mallard signs in to Feishu with Mallard's own app. Nothing runs on
Mallard's side: the desktop talks to Feishu directly, the sign-in it keeps
lives on the computer, and Mallard Cloud holds nothing about Feishu. The
first part is for the Feishu admin of an organization that wants to use
Mallard; the second is for Mallard's own maintainers.

## For your organization's Feishu admin

1. Install Mallard's app from the Feishu app store (or approve it when a
   user requests it). Mallard is a store app, so it reaches your tenant
   only once you have installed it.
2. Make it available to the users or departments that will use Mallard.
   A user the app is not available to cannot complete the sign-in; Mallard
   then says "Feishu denied access · ask your admin to install Mallard's
   app for you".
3. That is all. Users select **Connect Feishu** in Mallard, approve
   the six permissions with their own account, and choose a folder. No
   App ID or App Secret is typed anywhere.

What consent covers: Feishu's user consent covers every Drive folder the
user can reach, not only the folder Mallard uses. The sign-in Mallard keeps
on a computer can read and write those folders. Mallard writes only below
the storage's folder; the folder choice is Mallard's convention, not a
limit Feishu enforces on the token.

Sharing a storage between computers: a storage is one Drive folder. Share
that folder in Feishu with each person who will connect, and each signs
in with their own account; on the second computer, choose **Existing
folder** on the Folder line and paste the folder's link. Two computers
never share a sign-in.

## For Mallard's maintainers: the app

Mallard's app is created once in the
[Feishu Open Platform developer console](https://open.feishu.cn/app).
Until it is published as a store app it is a self-built app that reaches
Mallard's own tenant only, which is enough for the live checks.

Permissions, on the **user** identity and never on the app's tenant
identity:

| Scope | Why |
| --- | --- |
| `space:document:retrieve` | List a folder's contents |
| `space:folder:create` | Create the storage's folders |
| `drive:file:upload` | Upload files, including large ones in parts |
| `drive:file:download` | Download files and verify them |
| `auth:user.id:read` | Show the signed-in user on the Account line |
| `offline_access` | Renew access without a new sign-in for up to a year |

Security settings: the one redirect URL
`http://127.0.0.1:53683/feishu/callback`, exactly.

Use the matched OAuth v3 endpoints:

- Authorize: `https://accounts.feishu.cn/oauth/v3/authorize`
- Token: `https://accounts.feishu.cn/oauth/v3/token`

The live check on 2026-09-24 passed token exchange and user-info lookup with
this pair, PKCE S256, and the six scopes above. The older
`/open-apis/authen/v1/authorize` route returned a code that the v3 token
endpoint rejected with error `20049`, although local PKCE checks passed.
Keep PKCE enabled. On the v3 consent page, select all six permissions;
some sensitive permissions, including offline access, can start unchecked.

Store release: the first release must be gray (at most 30 tenants, 200
in total; automatic or a one-to-five-day manual review), and a non-public
or full release needs Feishu's ISV review program. Non-public release does
not skip review.

### The App Secret in the build

Feishu's token endpoint requires `client_secret` for every
developer-created app, and PKCE supplements the secret rather than
replacing it. Mallard carries the App ID and App Secret in its release
build, read by `FeishuApp::built_in` with `option_env!`: set
`MALLARD_FEISHU_APP_ID` and `MALLARD_FEISHU_APP_SECRET` in the build's
environment (the release workflow takes them from the repository's GitHub
Actions secrets of the same names), or for a local build put the two
lines in `.env.feishu` at the repository root (git-ignored):

```text
MALLARD_FEISHU_APP_ID=cli_…
MALLARD_FEISHU_APP_SECRET=…
```

`npm run tauri dev` and `npm run tauri build` read that file for whichever
of the two names the environment leaves unset; a plain `cargo` command
needs them exported. Cargo recompiles the crate whenever either value
changes, so a stale secret never lingers in a build. A build without them
still offers Feishu and reports the missing app on the Sign in line; a
development build also reads the same two names from its environment at
run time. The file's name starts with `.env.` because Vite's dev server
refuses to serve such files; keep it that way.

This is the standard posture for a native app. RFC 6749 §2.1 classes an
installed application as a public client and assumes that "any client
authentication credentials included in the application can be extracted";
RFC 8252 §8.5 says secrets statically included in an app distributed to
many users "should not be treated as confidential secrets" and asks
authorization servers not to require one, which Feishu nevertheless does;
Google says the same of its installed apps ("the client secret is
obviously not treated as a secret"). The secret therefore identifies the
app and authenticates nothing; PKCE S256, the fixed loopback redirect, and
the user's own consent protect the sign-in, exactly as RFC 8252 lays out.
Feishu's v3 token contract does know a *Public Client* that sends no
secret and must use PKCE, but every app created in the developer console
is a *Confidential Client*; should Feishu ever let Mallard's app be
registered as a Public Client, drop the secret from the build and nothing
else changes. Never commit the secret, and never "hide" it with obfuscation, which
would only suggest it protects something. The three facts below must stay
true:

- **The app holds no tenant-identity scopes.** With the secret alone, an
  attacker can mint an app or tenant access token for Mallard's app, and it
  reaches no data, because every permission is granted to the user identity
  only. Never add a scope on the app identity.
- **The redirect is the loopback port only.** A leaked secret cannot
  complete a sign-in without receiving the authorization code, which Feishu
  delivers only to `127.0.0.1:53683` on the consenting user's own computer.
  Never register another redirect URL.
- **A refresh needs the user's document.** The refresh token lives in
  `~/.mallard/feishu_connections/<id>.json`, owner-only; reading it needs
  local access, which already exposes everything on that computer. It is
  kept the way Mallard keeps its other provider credentials (an owner-only
  file under `~/.mallard`), not in the OS credential store; moving all of
  them there is one repository-wide item, not a Feishu one.

If the secret must be rotated, rotate it in the console and ship a new
Mallard: older builds then fail to refresh with "Feishu refused Mallard's
app · update Mallard", and updating restores them without a new sign-in.

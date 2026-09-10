# Dropbox shared namespace POC

Updated: 2026-09-07.

The standalone POC tests two authorized users against one existing writable
Dropbox namespace. Its code and run instructions are in
[poc/dropbox-shared-team-namespace](../poc/dropbox-shared-team-namespace/README.md).

The 18 offline POC tests passed before the production implementation. The
application now also has a reusable local HTTP mock. Its tests cover setup,
project restore, Library publication, stale writes, and cleanup. No live
Dropbox result is recorded. A live run needs a local credentials file for two
test users. Do not commit that file or paste tokens into chat.

The POC is a diagnostic harness. It is not an application execution path.
Its optional team-token selection and simulated ambiguous-write read-back do
not define production behavior.

The implemented application contract is in
[Dropbox storage](DESIGN_DROPBOX_SHARED_TEAM_NAMESPACE_STORAGE.md):

- Two connection fields: namespace ID and user-linked access token.
- Existing shared or team namespace only; no personal home fallback.
- Dropbox upload limit: 150 MiB per object, inclusive; direct upload only.
- Unknown mutable-write results never count as a confirmed CAS success.
- Cleanup requires an acknowledged revision and verified content.
- Schema 1 throughout; clean cutover with no migration or legacy execution path.

Pending live checks: cross-user access, project and Library publication,
stale-writer rejection, token replacement, and conditional clear. Record the
actual results after running them. Do not infer live success from mock tests.

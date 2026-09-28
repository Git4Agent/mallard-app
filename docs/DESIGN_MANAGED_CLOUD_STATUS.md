# Mallard Cloud repository status

Local metadata and portable bundles keep schema 1.

`ProjectStorageLink.managed_cloud_repository_id` is optional. New Cloud links
save the repository ID after registration or connection. If the field is absent,
serialization omits it and sync skips the Cloud status checks. There is no data
migration or automatic backfill. Other storage providers use the existing path.

For a link with an ID, the Cloud adapter reads
`GET /v1/repositories/{id}/head`. The response contains the snapshot and a compact
`link` object with visibility, release state, revision, address, and permissions.
An active repository with no snapshot returns HTTP 200 with `head: null`.
A pinned repository that answers `not_found` or `repository_removed` reads as
an empty stream, or as the repository this account collaborates on for the
same bundle. A Push creates the repository when there is none. A Push or a
Pull saves the repository it used as the pin. A refresh saves nothing.
(`PLAN_CLOUD_REPOSITORY_GONE_PUSH_AGAIN.md`, 2026-09-26, replaced the rule
that a vanished destination is an error.)

Comparison carries the status to the existing toolbar and notice area. Every
other error stops Push and Pull in that view. Retry runs the comparison again.
Rejected sign-in opens storage settings. Local sessions remain available. The
server still checks access and revisions when operations run.

The adapter uses one head request per comparison. It retains the status in memory
and does not write status to local metadata. The separate toolbar summary and
release-history requests are removed. Overlapping activity refreshes share their
request. No timer or whole-account polling is added.

The service determines visibility from the current repository row. Retained public
content does not make a private repository public. A rejection applies only to
its exact private publication. The initial schema has a repository/time index for
the latest release request. No new migration file or schema version is added.

HTTP 410 with `repository_removed` means deletion is in progress. HTTP 404 remains
ambiguous by design: the repository was deleted or the caller lost access. The
client does not disclose a hidden private repository's state. Both read as
gone: the status is `deleting` or `gone`, and a collaborator who was removed
from a private repository pushes to a private repository of their own. A `404`
that does not carry the service's own code is not that answer.

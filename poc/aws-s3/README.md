# AWS S3 feasibility for Mallard

Research date: 2026-10-05.

**AWS S3 support is feasible. Two transport and setup issues must be fixed before release.**
The local POC uses the current Mallard transport and publication code. It verifies the
main operations and reproduces both issues. It does not connect to AWS.

Mallard already depends on `aws-sdk-s3` 1.137.0. Its
[S3ObjectStore](../../crates/mallard-infrastructure/src/storage_core/s3_store.rs)
accepts a region, endpoint, bucket, and credentials. Both project sync and the Library
use this transport. The
[provider registry](../../crates/mallard-infrastructure/src/storage_providers/registry.rs)
has no AWS provider. The R2 adapter accepts only Cloudflare endpoints and uses region
`auto`. An AWS endpoint cannot be entered as an R2 destination.

AWS supports `PutObject` with `If-None-Match: *`. An existing object produces HTTP 412.
S3 also provides strong read and list consistency. These contracts fit Mallard's
immutable objects, parent claims, and publication records.
[AWS conditional writes](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html),
[AWS consistency](https://aws.amazon.com/s3/consistency/).

| Area | Current result | Required work |
| --- | --- | --- |
| Object transport | Local POC passes PUT, GET, SHA-256 checks, pagination, and claims. | Reuse `S3ObjectStore`. |
| Provider setup | No AWS provider is registered. | Add an `aws_s3` adapter and descriptor. |
| Authentication | Static keys work in the local fixture. `from_client` also sends a session token. | Add AWS profile loading and credential refresh through `aws-config`. |
| Publication conflict | HTTP 409 can produce a false success. | Separate HTTP 409 from HTTP 412. |
| Setup probe | Its first PUT is unconditional. | Make probe creation conditional. |
| Portable data | Existing object keys and schemas can be reused. | No portable schema change is expected. |

**The HTTP 409 defect is a release blocker.**
`put_absent_async` maps both HTTP 409 and HTTP 412 to `PreconditionFailed`.
`put_record_async` then returns `AlreadyPresent`. In
[publication::send_once](../../crates/mallard-infrastructure/src/storage_core/publication.rs),
an absent record is added to the returned view after any successful transport result.
The POC sends a real SDK request, returns `409 ConditionalRequestConflict`, and keeps
the record absent. Mallard still returns `WriteOutcome::Stored`.

AWS documents HTTP 409 when a concurrent delete wins. Thus, HTTP 409 does not prove
that the record exists. Return a conflict/error and use the existing read-only
settlement. Preserve one claim request and one record request. Do not add an automatic
publication retry. This issue is in shared S3/R2 code; the POC does not prove that a
live R2 service produces this response.
[AWS conflict behavior](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html).

**The probe must also support conditional-write policies.**
AWS lets a bucket policy require an `If-None-Match` header. Mallard's normal immutable
writes send this header, but `probe_put_async` does not. The POC returns HTTP 403 for
an unconditional PUT and reproduces a failure at the probe's Create stage. Reuse the
probe with conditional creation, provider-neutral text, and correct 409/412 handling.
[AWS policy support](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes-enforce.html).

Recommended first release: connect an existing, private, general purpose bucket.
Ask for **bucket name, region, and local AWS profile name**. Let the AWS SDK resolve
the regional endpoint. Keep the selected profile explicit so an unrelated environment
credential cannot silently replace it. Use the SDK profile provider for SSO or role
credentials and refresh. Keep credentials out of portable data. Report an expired
login with a clear sign-in action. The current dependency set has no `aws-config`.
[AWS Rust credential providers](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/credproviders.html).

Manual keys are another supported design option, but require a secret access key and
an optional secret session token. A pasted session token expires and does not provide
automatic refresh. The POC verifies token transmission only. It does not verify AWS
authentication, SSO, STS, or credential refresh. AWS recommends temporary credentials
for users.
[AWS authentication guidance](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/credentials.html).

For an existing dedicated bucket, normal sync needs `s3:ListBucket` on the bucket and
`s3:GetObject` plus `s3:PutObject` on `.mallard/*`. It does not need account-wide bucket
listing or bucket creation. `s3:DeleteObject` is needed for probe cleanup if cleanup
must succeed. The current probe accepts failed cleanup. A bucket using SSE-KMS also
needs the applicable KMS permissions and key policy. Use the bucket's default SSE-S3
encryption for the first live test.
[ListObjectsV2 permissions](https://docs.aws.amazon.com/AmazonS3/latest/API/API_ListObjectsV2.html),
[GetObject permissions](https://docs.aws.amazon.com/AmazonS3/latest/API/API_GetObject.html),
[PutObject and KMS permissions](https://docs.aws.amazon.com/AmazonS3/latest/API/API_PutObject.html).

Limit the first release to general purpose buckets. Directory buckets do not support
Mallard's current `StartAfter` listing or path-style requests, and their listings are
not ordered by key. Keep storage-wide Clear unavailable until its versioning and
retention behavior is defined. A normal DELETE in a versioned bucket adds a delete
marker and retains previous versions. Lifecycle expiry must not remove active
Mallard records or payloads. S3 charges for storage, requests, and applicable data
transfer; estimate these from the selected region and workload.
[Directory bucket limits](https://docs.aws.amazon.com/AmazonS3/latest/API/API_ListObjectsV2.html),
[Versioned deletion](https://docs.aws.amazon.com/AmazonS3/latest/userguide/DeletingObjectVersions.html),
[S3 pricing](https://aws.amazon.com/s3/pricing/).

Run the [local POC](../../crates/mallard-infrastructure/examples/aws_s3_poc.rs) from the repository root:

```sh
cargo run --locked --offline -p mallard-infrastructure --example aws_s3_poc
cargo test --locked --offline -p mallard-infrastructure --lib storage_core::s3_store::tests
```

Recorded results:

- Five POC scenarios passed: static credentials, session-token credentials, normal
  publication, rejected publication without resend, and listing settlement after a
  stored record receives an error response.
- Two defects were reproduced: false success on HTTP 409 and probe rejection under
  a conditional-write policy.
- All 25 existing S3 transport tests passed.

The POC's success exit means its checks and defect reproductions matched the current
code. It is not an AWS readiness result. After a fix, replace the defect assertions
with regression assertions that require the safe result.

The fixture binds only to `127.0.0.1`. It uses fixed fake credentials and synthetic
content. It checks the SigV4 region/service scope, token header, and SHA-256 checksum.
It does not verify the cryptographic signature. No real provider homes or Mallard
state are used. No production files or dependency manifests were changed for this POC.

Before release, fix the two defects, add the adapter, and run one live test against a
dedicated AWS bucket. Use temporary provider homes and synthetic project and Library
data. Verify Push/Pull, competing writers, denied permissions, wrong region, credential
expiry/refresh, and conditional-write policy enforcement. Then run the full frontend
and Rust checks required by the repository. Live AWS, native UI, KMS, and versioned
cleanup remain unverified.

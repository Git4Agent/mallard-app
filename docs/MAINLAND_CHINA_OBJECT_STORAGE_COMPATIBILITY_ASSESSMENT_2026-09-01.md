**MALLARD | TECHNICAL RESEARCH** · **MAINLAND CHINA OBJECT STORAGE**

# Mainland China Object Storage Compatibility Assessment

*Provider research and implementation feasibility for Mallard*

**Prepared for:** Mallard maintainers

**Research date:** 1 September 2026

**Scope:** Mainland China cloud leaders and prominent object-storage specialists

**Method:** Official provider documentation, current market research, and repository inspection

**Change status:** Research only; no product, schema, or safety change

---

## Executive summary

> **Overall result:** Mallard supports none of the reviewed mainland China providers through its current product path. The frontend and backend accept only canonical Cloudflare R2 configurations. The underlying S3 transport can become reusable, but every provider must pass Mallard's stronger atomic-write and integrity contract.

- **Preferred first pilot:** Baidu AI Cloud BOS. Its official documentation provides the closest match to Mallard's current protocol: SigV4, both bucket-addressing styles, ListObjectsV2, and conditional PutObject semantics.
- **Preferred second pilot:** Volcengine TOS. It requires virtual-host addressing and a live test of standard conditional-header and checksum behavior.
- **Candidates pending proof:** Qiniu Kodo and JD Cloud OSS document strong core S3 and Rust SDK compatibility, but not the exact compare-and-swap contract.
- **Top-three provider constraint:** Alibaba Cloud OSS, Huawei Cloud OBS, and Tencent Cloud COS do not establish the atomic mutable-head behavior that Mallard requires. Huawei also has a documented ListObjectsV2 gap.

Recommendation: generalize the R2-only setup into a verified S3-compatible path, then pilot BOS and TOS. Keep all other providers disabled until the same live conformance suite passes. If top-three mainland provider coverage is mandatory, evaluate an append-only publication protocol instead of weakening compare-and-swap safety.

## 1. Scope and method

### 1.1 Meaning of 'top provider'

No current public report was found for mainland China object-storage market share alone. This assessment therefore uses the current mainland China cloud-infrastructure leaders and adds prominent storage specialists that publish S3 compatibility documentation.

Omdia reported Q3 2025 cloud-infrastructure shares of 36% for Alibaba Cloud, 16% for Huawei Cloud, and 9% for Tencent Cloud. The market definition includes compute, storage, databases, and related cloud infrastructure. It is not an object-storage-only ranking. [MK1]

### 1.2 Evidence standard

- Use official provider API, endpoint, SDK, and release documentation for protocol claims.
- Treat marketing statements about S3 compatibility as discovery evidence, not proof of conditional writes or checksum behavior.
- State 'not documented' or 'not established' when the reviewed sources do not prove a behavior.
- Separate current Mallard product support from lower-level transport feasibility.
- Require a credentialed real-bucket test before Mallard enables a provider profile.

## 2. Mallard's current storage contract

### 2.1 Product restrictions

The current frontend accepts only Cloudflare R2 account and bucket URL forms. The backend then requires every S3 entry to use one canonical Cloudflare R2 destination. A manually edited non-R2 record is not a supported workaround because normal validation rejects it. [M1] [M2]

### 2.2 Current S3 client behavior

- The client uses the AWS Rust S3 SDK with an explicit endpoint.
- It uses a static access-key ID and secret-access key. It has no session-token field.
- It stores region 'auto' through the R2 setup path.
- It always forces path-style bucket addressing.
- Every PutObject request includes an AWS S3 SHA-256 checksum.
- Every download is also verified against Mallard's expected size and SHA-256 digest. [M3] [M5]

### 2.3 Compatibility gate

Basic upload and download support is not sufficient. A provider must pass all of these checks:

1. Authenticate with AWS SigV4 or a fully compatible request scheme using the configured endpoint and region.
1. Support the configured path-style or virtual-host bucket-addressing mode.
1. Support GetObject, HeadObject, PutObject, DeleteObject, and bounded metadata reads.
1. Support ListObjectsV2 with prefix, StartAfter, continuation tokens, IsTruncated, and stable key traversal.
1. Create a missing object atomically with If-None-Match: *.
1. Replace an object atomically with If-Match using its current opaque ETag.
1. Reject a stale ETag with HTTP 409 or 412.
1. Return a usable ETag from GET and HEAD responses.
1. Validate the AWS SHA-256 upload checksum or allow an immediate verified SHA-256 readback before success.
1. Provide read-after-write and delete visibility that passes Mallard's capability probe.

Mallard's existing setup probe already performs conditional create, ETag capture, matching replacement, required stale-ETag rejection, cleanup, and absence verification. The bundle engine uses the same ETag compare-and-swap rule for mutable heads and catalog tags. [M4] [M7]

> **Safety boundary:** Do not emulate compare-and-swap with HeadObject followed by an unconditional PutObject. Another client can update the object between those requests. Do not use ETag as the content checksum. Do not save a provider configuration when the live capability probe fails.

## 3. Provider compatibility summary

*All 'candidate' statuses remain conditional. No mainland provider has passed a live Mallard conformance test in this repository.*

| Provider | Addressing | Listing | Atomic conditional PUT | Assessment |
| --- | --- | --- | --- | --- |
| Alibaba Cloud OSS | Virtual only | ListObjectsV2 | Standard conditions documented unsupported | Blocked by protocol mismatch |
| Huawei Cloud OBS | Virtual only | ListObjects v1; V2 gap | Not established | Blocked by protocol gaps |
| Tencent Cloud COS | Virtual for new buckets | V2 needs live proof | Create-only vendor control; update CAS not established | Blocked pending atomic-write proof |

## 3. Provider compatibility summary - continued

| Provider | Addressing | Listing | Atomic conditional PUT | Assessment |
| --- | --- | --- | --- | --- |
| Baidu AI Cloud BOS | Path and virtual | ListObjectsV2 | Documented in native PutObject | Preferred first pilot |
| Volcengine TOS | Virtual only | ListObjectsV2 on regular buckets | Service support documented; S3 mapping needs proof | Preferred second pilot |
| Qiniu Kodo | Path and virtual | ListObjectsV2 | Not established | Candidate pending conformance evidence |
| JD Cloud OSS | Bucket-as-host documented | ListObjectsV2 | Not established | Candidate pending conformance evidence |
| China Mobile EOS | Path style in sample | Modern V2 proof absent | Not established | Exploratory only |
| China Telecom ZOS | Path and virtual forms documented | V2 proof absent | Not established | Exploratory only |
| China Unicom BOSS/OSS | Not established | Not established | Not established | Exploratory only |

*Assessment basis: Mallard contract [M1-M7] and official provider sources [A1-CU1]. 'Not established' means that the reviewed public documentation does not prove the required behavior.*

## 4. Detailed provider findings

### 4.1 Blocked by the current publication protocol

#### Alibaba Cloud OSS

**Blocked by documented protocol mismatch.**

OSS supports a documented subset of S3 operations, including core object CRUD and ListObjectsV2. It requires virtual-host addressing. Standard conditional PutObject headers such as If-Match and If-None-Match are explicitly unsupported. OSS provides a vendor create-only control, x-oss-forbid-overwrite, but the reviewed sources do not provide destination ETag compare-and-swap for an existing object. [A1] [A2] [A3]

Mallard implication: configurable virtual-host routing would solve only one problem. An adapter could map immutable create to the vendor header on a non-versioned bucket, but it could not safely implement Mallard's documented mutable-head replacement from the reviewed API contract.

#### Huawei Cloud OBS

**Blocked by documented protocol gaps.**

OBS requires virtual-host addressing for S3 access. The reviewed Huawei documentation states that standard OBS does not support ListObjectsV2. Its PutObject reference does not establish atomic If-Match or If-None-Match writes for destination objects. [H1] [H2] [H3]

Mallard implication: a provider adapter would need ListObjects v1 pagination and Huawei-compatible request handling. The publication protocol would still need an atomic-write design that the documented OBS API can implement.

#### Tencent Cloud COS

**Blocked pending atomic-write proof.**

COS supports AWS SigV4 and S3-style access. Buckets created from 1 January 2024 require virtual-host addressing. COS documents x-cos-forbid-overwrite as a create-only control for non-versioned buckets. The reviewed PutObject documentation does not establish exact destination ETag overwrite compare-and-swap. Tencent also states that COS does not guarantee complete S3 compatibility. [T1] [T2]

Mallard implication: core S3 access is plausible after virtual-host support, but the provider must not be enabled until live tests prove ListObjectsV2 and exact stale-ETag rejection. The vendor create-only header is not sufficient for mutable heads.

### 4.2 Preferred pilots

#### Baidu AI Cloud BOS

**Preferred first pilot.**

BOS documents stock AWS S3 SDK use, AWS SigV4, path-style and virtual-host addressing, core object operations, and ListObjectsV2. Native PutObject documentation explicitly supports If-Match, If-None-Match: *, ETags, and HTTP 412 on a failed precondition. [B1] [B2] [B3] [B4]

Required proof: the S3 endpoint must accept the AWS Rust SDK request, enforce all four conditional-write cases, return usable ETags, and either validate the AWS SHA-256 checksum header or pass an immediate SHA-256 readback. Use a mainland HTTPS endpoint and the provider's actual signing region.

#### Volcengine TOS

**Preferred second pilot.**

TOS provides S3 endpoints but requires virtual-host addressing. A July 2025 release note added If-Match and If-None-Match support to PutObject, CopyObject, and CompleteMultipartUpload. TOS also documents S3 V4 streaming with trailing headers. [V1] [V2]

Required proof: the S3 endpoint must map the standard conditional headers to the documented service behavior, reject a stale ETag with 409 or 412, and accept the AWS checksum mode used by Mallard. Use a regular flat/FNS bucket. Do not use an HNS bucket for the first pilot.

### 4.3 Candidates pending conformance evidence

#### Qiniu Kodo

**Candidate pending conformance evidence.**

Kodo documents path-style and virtual-host addressing, AWS SigV2 and SigV4, core object APIs, and ListObjectsV2. Qiniu also provides an official stock aws-sdk-s3 Rust example. Exact conditional PutObject and modern AWS checksum behavior are not sufficiently documented. [Q1] [Q2] [Q3]

#### JD Cloud OSS

**Candidate pending conformance evidence.**

JD Cloud documents SigV4, core S3 object operations, ListObjectsV2, endpoints, and a stock Rust AWS SDK guide. The reviewed PutObject reference does not establish If-Match, If-None-Match, or required 412 behavior. Flexible-checksum behavior also remains unproven. [J1] [J2] [J3] [J4]

### 4.4 Exploratory providers

#### China Mobile Cloud EOS

**Exploratory only.**

The product page states that EOS provides an S3 interface. The available official sample uses a vendor AWS SDK fork, SigV2, and forced path-style access. The reviewed public evidence does not establish stock Rust AWS SDK SigV4, conditional PutObject, or modern checksum behavior. [C1] [C2]

#### China Telecom Tianyi Cloud ZOS

**Exploratory only.**

ZOS publishes current S3-style APIs, endpoint forms, and x-amz-content-sha256 usage. The reviewed documentation does not establish the complete Mallard contract for stock AWS SigV4, ListObjectsV2 pagination, conditional writes, and flexible checksums. [CT1] [CT2]

#### China Unicom Cloud BOSS/OSS

**Exploratory only.**

The product page claims Amazon S3 compatibility, but the reviewed public protocol evidence is insufficient. Endpoint, signing, addressing, ListObjectsV2, conditional writes, and checksum behavior all require provider confirmation and testing. [CU1]

## 5. Recommended implementation sequence

> **Near-term recommendation:** Build one verified generic S3 path and use the same strict capability probe for every provider. Do not add provider names to the UI before the live probe passes.

1. Generalize the R2-only setup into an S3-compatible storage profile. Keep R2 account provisioning as a provider-specific flow.
1. Store the endpoint, bucket, actual signing region, and path-style or virtual-host addressing mode.
1. Keep static AK/SK support for the first slice. Add a session-token field only if temporary credentials are explicitly in scope.
1. Make S3 client addressing configurable instead of always forcing path style.
1. Generalize the existing strict R2 capability probe. Preserve conditional create, ETag capture, matching replacement, stale rejection, checksum verification, deletion, and absence verification.
1. Add real temporary-bucket integration tests for signing, TLS, DNS, pagination, consistency, checksums, and error status mapping.
1. Pilot Baidu BOS. Enable it only when the full conformance suite passes.
1. Add virtual-host support and pilot Volcengine TOS. Then test Qiniu Kodo and JD Cloud with the same suite.
1. Keep Alibaba OSS, Huawei OBS, and Tencent COS disabled under the current mutable-head publication protocol.
1. If top-three mainland provider coverage is a product requirement, evaluate append-only immutable head records and deterministic reconciliation as a separate protocol project.

## 6. Conformance test checklist

- **Authentication:** Use the exact endpoint, signing region, static credentials, service name, and HTTPS certificate path that Mallard will ship.
- **Addressing:** Verify both configured routing and bucket naming. Confirm path-style or virtual-host behavior without redirects that invalidate signatures.
- **Core object operations:** Put, Get, Head, and Delete a zero-byte object and a normal object. Require usable ETags and HTTP 404 for missing reads.
- **Atomic create:** Create with If-None-Match: *. Repeat the request and require HTTP 409 or 412.
- **Atomic replacement:** Replace with the current If-Match ETag. Then reuse the stale ETag and require HTTP 409 or 412.
- **Integrity:** Confirm that the AWS SHA-256 checksum is validated. If it is not supported, require immediate SHA-256 readback before success.
- **Listing:** Test ListObjectsV2 prefix filtering, StartAfter, continuation tokens, IsTruncated, stable ordering, and pages larger than 1,000 keys.
- **Visibility:** Verify immediate keyed read after write and HEAD 404 after delete. Treat delayed visibility as a failed provider profile.
- **Key behavior:** Test Mallard's .mallard/ namespace, Unicode keys, long keys, lexical neighbors, and exact-prefix filtering.
- **Failure resolution:** Test timeouts, ambiguous write outcomes, stale conditions, missing ETags, and cleanup failure without weakening revision or digest checks.

## 7. Decision and next actions

| Decision | Recommended action | Acceptance condition |
| --- | --- | --- |
| Baidu BOS pilot | Approve | Generic S3 setup and full live conformance suite pass |
| Volcengine TOS pilot | Approve after virtual-host support | Standard conditional headers and checksum mode pass |
| Qiniu and JD evaluation | Defer until common probe exists | Same provider-neutral suite passes without safety exceptions |
| Alibaba, Huawei, Tencent support | Do not enable under current protocol | Documented atomic publication primitive or approved protocol redesign |

*Decision basis: current Mallard contract [M1-M7] and provider findings in Sections 3 and 4.*

## Appendix A - Source register

*External protocol claims use official provider sources. The market-context source is an independent analyst report. Mallard references point to the inspected repository state on 1 September 2026.*

### Market context

**[MK1]** [Omdia: Mainland China's cloud infrastructure market accelerates to 24% growth in Q3 2025](https://omdia.tech.informa.com/pr/2026/feb/mainland-chinas-cloud-infrastructure-market-accelerates-to-24percent-growth-in-q3-2025) - Mainland cloud-infrastructure market shares and market definition.

### Mallard repository references

**[M1]** [src/components/project-sync/StorageSettings.tsx:51](../src/components/project-sync/StorageSettings.tsx#L51) - Frontend R2 endpoint parsing and R2-only storage form behavior.

**[M2]** [crates/mallard-application/src/storage_setup.rs:388](../crates/mallard-application/src/storage_setup.rs#L388) - Backend validation requires every S3 storage to use a canonical Cloudflare R2 destination.

**[M3]** [crates/mallard-infrastructure/src/storage_core/s3_store.rs:1277](../crates/mallard-infrastructure/src/storage_core/s3_store.rs#L1277) - AWS S3 client construction, static credentials, region, endpoint, and forced path-style addressing.

**[M4]** [crates/mallard-infrastructure/src/storage_core/s3_store.rs:269](../crates/mallard-infrastructure/src/storage_core/s3_store.rs#L269) - Capability probe for conditional create, ETag replacement, stale-write rejection, and cleanup.

**[M5]** [crates/mallard-infrastructure/src/storage_core/s3_store.rs:665](../crates/mallard-infrastructure/src/storage_core/s3_store.rs#L665) - Conditional PutObject and AWS SHA-256 checksum behavior.

**[M6]** [crates/mallard-infrastructure/src/storage_core/s3_store.rs:849](../crates/mallard-infrastructure/src/storage_core/s3_store.rs#L849) - ListObjectsV2 prefix, StartAfter, continuation-token, and pagination behavior.

**[M7]** [crates/mallard-infrastructure/src/bundle_engine.rs:915](../crates/mallard-infrastructure/src/bundle_engine.rs#L915) - Immutable object publication and mutable-head compare-and-swap flow.

### Alibaba Cloud OSS

**[A1]** [Amazon S3 compatibility](https://www.alibabacloud.com/help/en/oss/developer-reference/compatibility-with-amazon-s3) - Supported operations, virtual-host requirement, and ETag differences.

**[A2]** [Conditional PutObject limitation](https://www.alibabacloud.com/help/en/oss/user-guide/0017-00000245) - Standard PutObject conditional headers are not implemented.

**[A3]** [PutObject](https://www.alibabacloud.com/help/en/oss/developer-reference/putobject) - Vendor no-overwrite control and object-write behavior.

### Tencent Cloud COS

**[T1]** [S3-compatible application configuration](https://intl.cloud.tencent.com/document/product/436/34688?lang=en) - Endpoint, signatures, virtual-host requirement for new buckets, and compatibility caveat.

**[T2]** [PutObject](https://cloud.tencent.com/document/product/436/7749) - Vendor no-overwrite header and normal overwrite behavior.

### Huawei Cloud OBS

**[H1]** [AWS S3 access guidance](https://support.huaweicloud.com/obs_faq/obs_faq_0274.html) - AWS SigV4 and checksum-mode guidance.

**[H2]** [ListObjectsV2 limitation](https://support.huaweicloud.com/intl/en-us/mrs_faq/mrs_03_1102.html) - Standard OBS ListObjectsV2 limitation.

**[H3]** [PutObject](https://support.huaweicloud.com/intl/en-us/api-obs/obs_04_0080.html) - OBS PutObject request and response behavior.

### Baidu AI Cloud BOS

**[B1]** [S3 endpoints and addressing](https://cloud.baidu.com/doc/BOS/s/xjwvyq9l4) - Path-style and virtual-host endpoint forms.

**[B2]** [AWS SDK access guide](https://cloud.baidu.com/doc/BOS/s/ojwvyq973) - AWS SigV4 and SDK configuration.

**[B3]** [S3 compatibility matrix](https://intl.cloud.baidu.com/en/doc/BOS/s/Njwvyq8ho-intl-en) - ListObjectsV2 and core S3 operations.

**[B4]** [Conditional PutObject](https://cloud.baidu.com/doc/BOS/s/Ikc5nv3wc) - If-Match, If-None-Match: *, ETag, 412, and native SHA-256 header behavior.

### Volcengine TOS

**[V1]** [S3 virtual-host addressing requirement](https://www.volcengine.com/docs/6349/1288493?lang=zh) - TOS S3 endpoint addressing behavior.

**[V2]** [Conditional-write release note](https://www.volcengine.com/docs/6349/79878?lang=en) - July 2025 addition of If-Match and If-None-Match for upload operations.

### Qiniu Kodo

**[Q1]** [S3 access domain names](https://developer.qiniu.com/kodo/4088/s3-access-domainname) - Path-style and virtual-host endpoint forms.

**[Q2]** [S3 compatibility matrix](https://developer.qiniu.com/kodo/4087/compatible-s3-api) - Supported core APIs and ListObjectsV2.

**[Q3]** [AWS Rust SDK example](https://developer.qiniu.com/kodo/12572/aws-sdk-rust-examples) - Stock aws-sdk-s3 integration example.

### JD Cloud OSS

**[J1]** [Regions and endpoints](https://docs.jdcloud.com/cn/object-storage-service/api/regions-and-endpoints) - Regional S3 endpoint forms.

**[J2]** [S3 compatibility overview](https://docs.jdcloud.com/cn/object-storage-service/api/compatibility-api-overview) - Core object APIs and ListObjectsV2 mapping.

**[J3]** [Rust SDK guide](https://docs.jdcloud.com/cn/object-storage-service/sdk-rust) - Stock aws-sdk-s3 configuration.

**[J4]** [PutObject](https://docs.jdcloud.com/cn/object-storage-service/api/put-object-2) - Published PutObject request behavior; exact conditional headers are not established.

### Other mainland providers

**[C1]** [China Mobile Cloud EOS product page](https://qiye.cmcloud.com/main/zy-eos.html) - Product-level S3 interface statement.

**[C2]** [China Mobile official Java SDK sample](https://gitee.com/chinamobile_ecloud/ecloud-eos-sdk-java/blob/master/src/main/java/com/chinamobile/cmss/eos/s3/InitializeSample.java) - Vendor SDK fork, SigV2, endpoint, and forced path-style example.

**[CT1]** [China Telecom ZOS documentation](https://www.ctyun.cn/document/10026735/) - Current ZOS object-storage documentation.

**[CT2]** [China Telecom ZOS PutObject](https://www.ctyun.cn/document/10026735/11070698) - PutObject and x-amz-content-sha256 use.

**[CU1]** [China Unicom BOSS product page](https://www.cucloud.cn/product/BOSS.html) - Product-level Amazon S3 compatibility statement; protocol detail remains insufficient.

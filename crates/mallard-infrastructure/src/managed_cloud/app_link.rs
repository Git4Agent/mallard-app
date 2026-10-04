//! The link the website hands the desktop: `mallard://open?address=<a
//! page's address>`. Three pages are known: a repository's (Open in
//! Mallard), one session's of a repository (Pull in Mallard), and the
//! listing (Share yours). It names a page and nothing else: no verb beyond
//! `open`, no folder, no account. Any page can fire the scheme, so whatever
//! is not exactly this form is refused, and only the host and `owner/name`,
//! with a session's provider and cloud thread, or the host and one of four
//! kinds, ever leave this module.
use super::address::{origin_host, parse_session_page_address, RepositoryAddress};
use mallard_domain::{CloudThreadId, ProviderKey};
use serde::Serialize;

/// The scheme registered for the app (`tauri.conf.json`).
pub const APP_LINK_SCHEME: &str = "mallard";

/// The longest link read; the address parser holds its own input to the
/// same length.
const MAX_LINK_LEN: usize = 2048;

/// The listing's path on the website (`LISTING_PATH` in its `links.ts`).
/// GitHub reserves the name, so no Owner login can take it.
const LISTING_PATH: &str = "/repositories";

/// The kinds the website's listing filters by: its `?type=` values.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ListingKind {
    Project,
    AgentSkill,
    PluginInstallIntent,
    DesktopPet,
}

impl ListingKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "project" => Some(Self::Project),
            "agent_skill" => Some(Self::AgentSkill),
            "plugin_install_intent" => Some(Self::PluginInstallIntent),
            "desktop_pet" => Some(Self::DesktopPet),
            _ => None,
        }
    }
}

/// A page on a Mallard Cloud host. `host` is spelled as
/// `address::origin_host` spells one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppLink {
    /// A repository's page.
    Repository {
        host: String,
        address: RepositoryAddress,
    },
    /// One session's page: its repository, and the session's provider and
    /// cloud thread, as the page's `sessions/{id}` tail names them.
    Session {
        host: String,
        address: RepositoryAddress,
        provider: ProviderKey,
        cloud_thread_id: CloudThreadId,
    },
    /// The listing, and the kind its chips were filtered to.
    Listing {
        host: String,
        kind: Option<ListingKind>,
    },
}

/// Read a link, or `None` when it is not one this app opens.
pub fn parse_app_link(link: &str) -> Option<AppLink> {
    if link.len() > MAX_LINK_LEN || link.chars().any(char::is_whitespace) {
        return None;
    }
    let url = url::Url::parse(link).ok()?;
    if url.scheme() != APP_LINK_SCHEME
        || !url
            .host_str()
            .is_some_and(|verb| verb.eq_ignore_ascii_case("open"))
        || !matches!(url.path(), "" | "/")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let mut pairs = url.query_pairs();
    let (key, page) = pairs.next()?;
    if key != "address" || pairs.next().is_some() || page.chars().any(char::is_whitespace) {
        return None;
    }
    if let Some(listing) = parse_listing(&page) {
        return Some(listing);
    }
    Some(match parse_session_page_address(&page).ok()? {
        (host, address, None) => AppLink::Repository { host, address },
        (host, address, Some((provider, cloud_thread_id))) => AppLink::Session {
            host,
            address,
            provider,
            cloud_thread_id,
        },
    })
}

/// The website's listing, `http(s)://{host}/repositories`. Its `type` is
/// the one filter read, against the four kinds; any other key is dropped,
/// and a `type` that is not one of them, or given twice, is not a link.
fn parse_listing(page: &str) -> Option<AppLink> {
    let url = url::Url::parse(page).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path().trim_end_matches('/') != LISTING_PATH
    {
        return None;
    }
    let mut kind = None;
    for (key, value) in url.query_pairs() {
        if key == "type" {
            if kind.is_some() {
                return None;
            }
            kind = Some(ListingKind::parse(&value)?);
        }
    }
    Some(AppLink::Listing {
        host: origin_host(page)?,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(host: &str, owner: &str, name: &str) -> Option<AppLink> {
        Some(AppLink::Repository {
            host: host.into(),
            address: RepositoryAddress {
                owner: owner.into(),
                name: name.into(),
            },
        })
    }

    fn listing(host: &str, kind: Option<ListingKind>) -> Option<AppLink> {
        Some(AppLink::Listing {
            host: host.into(),
            kind,
        })
    }

    #[test]
    fn the_listings_link_reads_as_the_listing_and_its_kind() {
        // `listingAppLink()` in the website's pull.ts, verbatim.
        assert_eq!(
            parse_app_link("mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Frepositories"),
            listing("mallard-ai.com", None)
        );
        for (value, kind) in [
            ("project", ListingKind::Project),
            ("agent_skill", ListingKind::AgentSkill),
            ("plugin_install_intent", ListingKind::PluginInstallIntent),
            ("desktop_pet", ListingKind::DesktopPet),
        ] {
            assert_eq!(
                parse_app_link(&format!(
                    "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Frepositories%3Ftype%3D{value}"
                )),
                listing("mallard-ai.com", Some(kind)),
                "{value}"
            );
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::json!(value),
                "the window reads the website's own word"
            );
        }
        // The listing's other filters ride along on the page and are dropped.
        assert_eq!(
            parse_app_link("mallard://open?address=https%3A%2F%2Fwww.mallard-ai.com%2Frepositories%2F%3Fscope%3Dyours%26type%3Ddesktop_pet%26sort%3Dstars%26q%3Dduck"),
            listing("mallard-ai.com", Some(ListingKind::DesktopPet))
        );
        assert_eq!(
            parse_app_link("mallard://open?address=http%3A%2F%2Flocalhost%3A8787%2Frepositories"),
            listing("localhost:8787", None)
        );
        // A kind the listing does not have, twice, or empty: not a link.
        for refused in [
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Frepositories%3Ftype%3Dpush",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Frepositories%3Ftype%3D",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Frepositories%3Ftype%3Dproject%26type%3Ddesktop_pet",
            "mallard://open?address=https%3A%2F%2Fuser%40mallard-ai.com%2Frepositories",
            "mallard://open?address=ftp%3A%2F%2Fmallard-ai.com%2Frepositories",
            "mallard://open?address=mallard-ai.com%2Frepositories",
            // A path under the listing is neither the listing nor a repository.
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Frepositories%2FHorrorGame",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Frepositories%2FHorrorGame%3Ftype%3Dproject",
            // The website's other pages stay unknown.
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Fdocs",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Freviews",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Fprivacy",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2F",
        ] {
            assert_eq!(parse_app_link(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn the_websites_link_reads_as_its_page() {
        // `appLink()` in the website's pull.ts, verbatim.
        assert_eq!(
            parse_app_link(
                "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Frika%2Fmallard-with-pelican"
            ),
            link("mallard-ai.com", "rika", "mallard-with-pelican")
        );
        for spelling in [
            "mallard://open?address=https://mallard-ai.com/Rika321/HorrorGame",
            "mallard://open/?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame",
            "MALLARD://OPEN?address=https%3A%2F%2Fwww.mallard-ai.com%2FRika321%2FHorrorGame%2F",
            // A tail that is not one session's, and the page's query, are
            // dropped: the repository is what opens.
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Frelease%3Fcommit%3Dabc%26scope%3Dprivate",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2F",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2Fcodex%3Aconversation%3Acloudthread-0a1b%2Fraw",
        ] {
            assert_eq!(
                parse_app_link(spelling),
                link("mallard-ai.com", "Rika321", "HorrorGame"),
                "{spelling}"
            );
        }
        assert_eq!(
            parse_app_link(
                "mallard://open?address=http%3A%2F%2Flocalhost%3A8787%2Foctocat%2Fnotes"
            ),
            link("localhost:8787", "octocat", "notes")
        );
    }

    #[test]
    fn a_session_pages_link_reads_as_the_repository_and_the_session() {
        let session = |host: &str, owner: &str, name: &str, provider: &str, thread: &str| {
            Some(AppLink::Session {
                host: host.into(),
                address: RepositoryAddress {
                    owner: owner.into(),
                    name: name.into(),
                },
                provider: ProviderKey::parse(provider).unwrap(),
                cloud_thread_id: CloudThreadId::parse(thread).unwrap(),
            })
        };
        // `appLink(pageAddress)` in the website's pull.ts, verbatim: the id's
        // colons raw in the page address, and percent-encoded in it.
        for spelling in [
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2Fcodex%3Aconversation%3Acloudthread-0a1b2c3d",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2Fcodex%253Aconversation%253Acloudthread-0a1b2c3d",
            // The page's query is the page's, and is dropped.
            "mallard://open?address=https%3A%2F%2Fwww.mallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2Fcodex%3Aconversation%3Acloudthread-0a1b2c3d%3Fcommit%3D9f3c2d1%26scope%3Dprivate",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2Fcodex%253Aconversation%253Acloudthread-0a1b2c3d%2F%3Fcommit%3D4",
        ] {
            assert_eq!(
                parse_app_link(spelling),
                session(
                    "mallard-ai.com",
                    "Rika321",
                    "HorrorGame",
                    "codex",
                    "cloudthread-0a1b2c3d"
                ),
                "{spelling}"
            );
        }
        assert_eq!(
            parse_app_link("mallard://open?address=http%3A%2F%2Flocalhost%3A8787%2Foctocat%2Fnotes%2Fsessions%2Fclaude%3Aconversation%3Acloudthread-ff"),
            session("localhost:8787", "octocat", "notes", "claude", "cloudthread-ff")
        );
        // An id the website never writes is a forged or broken link, never
        // the repository: a local thread's id, another resource class, a
        // provider alone, an id that is not one, an encoded separator.
        for refused in [
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2Fres-1",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2Fcodex%3Aconversation%3Athread-0a1b",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2Fcodex%3Asession%3Acloudthread-0a1b",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2Fcodex",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2FCodex%3Aconversation%3Acloudthread-0a1b",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2Fcodex%3Aconversation%3Acloudthread-0a1b%252Fraw",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2FHorrorGame%2Fsessions%2Fcodex%3Aconversation%3Acloudthread-%25FF",
            // A session of what is not a repository page.
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Fdocs%2Fpull%2Fsessions%2Fcodex%3Aconversation%3Acloudthread-0a1b",
        ] {
            assert_eq!(parse_app_link(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn anything_else_is_not_a_link() {
        let long = format!(
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Fa%2F{}",
            "b".repeat(2048)
        );
        for refused in [
            "",
            "mallard://",
            "mallard://open",
            "mallard://open?",
            // Another verb, or a path under the verb.
            "mallard://pull?address=https%3A%2F%2Fmallard-ai.com%2Fa%2Fb",
            "mallard://open/pull?address=https%3A%2F%2Fmallard-ai.com%2Fa%2Fb",
            // More than the one parameter, the parameter twice, or another one.
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Fa%2Fb&now=1",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Fa%2Fb&address=https%3A%2F%2Fmallard-ai.com%2Fc%2Fd",
            "mallard://open?folder=%2Ftmp",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Fa%2Fb#top",
            "mallard://user@open?address=https%3A%2F%2Fmallard-ai.com%2Fa%2Fb",
            "mallard://open:80?address=https%3A%2F%2Fmallard-ai.com%2Fa%2Fb",
            // Another scheme.
            "https://mallard-ai.com/a/b",
            "mallard-dev://open?address=https%3A%2F%2Fmallard-ai.com%2Fa%2Fb",
            // An address that is not a repository page on a named host.
            "mallard://open?address=a%2Fb",
            "mallard://open?address=mallard-ai.com%2Fa%2Fb",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2F",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Frika",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Finvitations%2Fabc",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Fdocs%2Fpush-and-pull",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Freviews%2Fabc",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Fprivacy%2Fabc",
            "mallard://open?address=https%3A%2F%2Fgithub.com%2Fa%2Fb",
            "mallard://open?address=ftp%3A%2F%2Fmallard-ai.com%2Fa%2Fb",
            "mallard://open?address=file%3A%2F%2F%2Fetc%2Fpasswd",
            "mallard://open?address=https%3A%2F%2Fuser%40mallard-ai.com%2Fa%2Fb",
            "mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Fa%2Fb%20c",
            "mallard://open?address=https%3A%2F%2F%2Fa%2Fb",
            " mallard://open?address=https%3A%2F%2Fmallard-ai.com%2Fa%2Fb",
            long.as_str(),
        ] {
            assert_eq!(parse_app_link(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn a_look_alike_host_stays_its_own_host() {
        // Read, never trusted: no storage signs in there, so nothing is asked.
        assert_eq!(
            parse_app_link(
                "mallard://open?address=https%3A%2F%2Fmallard-ai.com.evil.example%2Fa%2Fb"
            ),
            link("mallard-ai.com.evil.example", "a", "b")
        );
    }
}

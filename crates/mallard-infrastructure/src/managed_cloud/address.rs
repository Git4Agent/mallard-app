//! A Mallard Cloud repository address as a person pastes it: the page
//! address from the website (`https://mallard-ai.com/Rika321/HorrorGame`,
//! with or without the scheme, with or without a `/sessions/…` tail), or
//! the bare `owner/name`. A name is any script, spelled raw
//! (`Rika321/大黄`) or percent-encoded (`Rika321/%E5%A4%A7%E9%BB%84`, what a
//! browser hands an app link): both read the same. The storage's own origin
//! is the only host accepted, so a staging address never reaches the
//! production service.
use super::names;
use mallard_domain::{parse_cloud_conversation_resource_id, CloudThreadId, ProviderKey};
use std::fmt;

/// `{owner}/{name}` as the service addresses a repository (section 4.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryAddress {
    pub owner: String,
    pub name: String,
}

impl fmt::Display for RepositoryAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddressError {
    /// The address names another Mallard Cloud host than the storage's.
    WrongService { host: String, expected: String },
    /// A `github.com` address: the Git remote, not the cloud repository.
    GitHub,
    /// Not an address at all.
    Invalid,
}

/// The website's own top-level paths, which are never an owner login. The
/// first six are the static routes in its `App.tsx`.
const RESERVED_OWNERS: [&str; 10] = [
    "repositories",
    "invitations",
    "reviews",
    "super-admin",
    "docs",
    "privacy",
    "v1",
    "auth",
    "login",
    "logout",
];

/// GitHub's login shape, as the service's `isLoginShaped` accepts it.
fn login_shaped(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The host (with its port when the origin carries one) an address must
/// name to be the storage's, lower-cased; `www.` is dropped on both sides.
pub fn origin_host(service_origin: &str) -> Option<String> {
    let url = url::Url::parse(service_origin).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

/// A pasted first segment is a host when it could not be a login: it holds
/// a dot or a port, or it is `localhost`.
fn looks_like_host(segment: &str) -> bool {
    segment.contains('.') || segment.contains(':') || segment.eq_ignore_ascii_case("localhost")
}

/// An address taken apart: the host it names, as `origin_host` spells one,
/// when it names one; whether it led with `http(s)://`; and the path.
struct Parts<'a> {
    host: Option<String>,
    schemed: bool,
    path: &'a str,
}

fn parts(input: &str) -> Result<Parts<'_>, AddressError> {
    let text = input.trim();
    if text.is_empty() || text.len() > 2048 || text.chars().any(char::is_whitespace) {
        return Err(AddressError::Invalid);
    }
    // Drop a query or fragment: the website's session page carries `?commit=`.
    let text = text.split(['?', '#']).next().unwrap_or_default();
    let (authority, schemed, path) = match text.split_once("://") {
        Some((scheme, rest)) => {
            if !(scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http")) {
                return Err(AddressError::Invalid);
            }
            let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
            (Some(authority), true, path)
        }
        None => {
            let (first, rest) = text.split_once('/').unwrap_or((text, ""));
            if looks_like_host(first) {
                (Some(first), false, rest)
            } else {
                (None, false, text)
            }
        }
    };
    let host = match authority {
        Some(authority) => {
            if authority.contains('@') {
                return Err(AddressError::Invalid);
            }
            let host = authority.to_ascii_lowercase();
            let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
            if host == "github.com" {
                return Err(AddressError::GitHub);
            }
            Some(host)
        }
        None => None,
    };
    Ok(Parts {
        host,
        schemed,
        path,
    })
}

/// Parse `input` against the storage's `service_origin`.
pub fn parse_repository_address(
    input: &str,
    service_origin: &str,
) -> Result<RepositoryAddress, AddressError> {
    let Parts { host, path, .. } = parts(input)?;
    if let Some(host) = host {
        let expected = origin_host(service_origin).ok_or(AddressError::Invalid)?;
        if host != expected {
            return Err(AddressError::WrongService { host, expected });
        }
    }
    repository_in(path)
}

/// A page's whole address taken apart: the host it names, and its path. The
/// scheme and the host are required, since no storage has been chosen yet to
/// check a host against.
fn page_parts(input: &str) -> Result<(String, &str), AddressError> {
    let Parts {
        host,
        schemed,
        path,
    } = parts(input)?;
    match host {
        Some(host) if schemed && !host.is_empty() => Ok((host, path)),
        _ => Err(AddressError::Invalid),
    }
}

/// A repository page's whole address, `http(s)://{host}/{owner}/{name}…`:
/// the host it names and the repository. What an app link carries, where no
/// storage has been chosen yet to check the host against.
pub fn parse_page_address(input: &str) -> Result<(String, RepositoryAddress), AddressError> {
    let (host, path) = page_parts(input)?;
    Ok((host, repository_in(path)?))
}

/// The session a session page names: its provider and its cloud thread.
pub type SessionAddress = (ProviderKey, CloudThreadId);

/// A page's whole address, read for the session it names as well:
/// `http(s)://{host}/{owner}/{name}/sessions/{id}`, where `{id}` is the
/// manifest row's id (`<provider>:conversation:cloudthread-…`), spelled raw
/// or percent-encoded. Any other tail (none, `/release`) names no session
/// and the page stays its repository's. A `sessions/{id}` whose id is not a
/// cloud conversation's is not an address: the website never writes one.
pub fn parse_session_page_address(
    input: &str,
) -> Result<(String, RepositoryAddress, Option<SessionAddress>), AddressError> {
    let (host, path) = page_parts(input)?;
    let address = repository_in(path)?;
    let mut tail = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .skip(2);
    let session = match (tail.next(), tail.next(), tail.next()) {
        (Some("sessions"), Some(id), None) => {
            let id = percent_encoding::percent_decode_str(id)
                .decode_utf8()
                .map_err(|_| AddressError::Invalid)?;
            Some(parse_cloud_conversation_resource_id(&id).map_err(|_| AddressError::Invalid)?)
        }
        _ => None,
    };
    Ok((host, address, session))
}

/// `{owner}/{name}` at the head of a page's path. Each segment is
/// percent-decoded on its own and the name read in NFC, so the raw and the
/// encoded spelling are one address; a segment that decodes to something
/// outside the rule (`/`, `%`, a space, a control) is not a name.
fn repository_in(path: &str) -> Result<RepositoryAddress, AddressError> {
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    let (Some(owner), Some(name)) = (segments.next(), segments.next()) else {
        return Err(AddressError::Invalid);
    };
    let (Some(owner), Some(name)) = (names::decode_segment(owner), names::decode_segment(name))
    else {
        return Err(AddressError::Invalid);
    };
    if !login_shaped(&owner)
        || !names::is_repository_name(&name)
        || RESERVED_OWNERS
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(&owner))
    {
        return Err(AddressError::Invalid);
    }
    Ok(RepositoryAddress { owner, name })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "https://mallard-ai.com";

    fn address(owner: &str, name: &str) -> Result<RepositoryAddress, AddressError> {
        Ok(RepositoryAddress {
            owner: owner.into(),
            name: name.into(),
        })
    }

    #[test]
    fn every_spelling_of_the_page_address_reads_the_same() {
        for input in [
            "https://mallard-ai.com/Rika321/HorrorGame",
            "https://mallard-ai.com/Rika321/HorrorGame/",
            "HTTPS://MALLARD-AI.COM/Rika321/HorrorGame",
            "https://www.mallard-ai.com/Rika321/HorrorGame",
            "https://mallard-ai.com/Rika321/HorrorGame/sessions/res-1?commit=abc",
            "https://mallard-ai.com/Rika321/HorrorGame/release#top",
            "mallard-ai.com/Rika321/HorrorGame",
            "www.mallard-ai.com/Rika321/HorrorGame/sessions/x",
            "Rika321/HorrorGame",
            "  Rika321/HorrorGame  ",
            "/Rika321/HorrorGame/",
        ] {
            assert_eq!(
                parse_repository_address(input, ORIGIN),
                address("Rika321", "HorrorGame"),
                "{input}"
            );
        }
        assert_eq!(
            parse_repository_address(
                "http://localhost:8787/octocat/Birding.Log",
                "http://localhost:8787"
            ),
            address("octocat", "Birding.Log")
        );
        assert_eq!(
            parse_repository_address("localhost:8787/octocat/x_y-z", "http://localhost:8787"),
            address("octocat", "x_y-z")
        );
    }

    #[test]
    fn a_name_of_any_script_reads_the_same_raw_and_encoded() {
        for input in [
            "Rika321/大黄",
            "Rika321/%E5%A4%A7%E9%BB%84",
            "mallard-ai.com/Rika321/大黄/",
            "https://www.mallard-ai.com/Rika321/%E5%A4%A7%E9%BB%84/sessions/x?commit=1",
            "https://mallard-ai.com/Rika321/%E5%A4%A7%E9%BB%84",
            "/Rika321/大黄/",
        ] {
            assert_eq!(
                parse_repository_address(input, ORIGIN),
                address("Rika321", "大黄"),
                "{input}"
            );
        }
        // A decomposed spelling reads in NFC; an encoded login is a login.
        assert_eq!(
            parse_repository_address("Rika321/Cafe%CC%81", ORIGIN),
            address("Rika321", "Caf\u{e9}")
        );
        assert_eq!(
            parse_repository_address("%52ika321/Caf\u{e9}", ORIGIN),
            address("Rika321", "Caf\u{e9}")
        );
        assert_eq!(
            parse_page_address("https://mallard-ai.com/Rika321/%E5%A4%A7%E9%BB%84/"),
            Ok((
                "mallard-ai.com".into(),
                RepositoryAddress {
                    owner: "Rika321".into(),
                    name: "大黄".into()
                }
            ))
        );
        // What decodes to something outside the rule is not an address.
        for input in [
            "Rika321/%E5%A4%A7%20%E9%BB%84",
            "Rika321/%E5%A4%A7%2F%E9%BB%84",
            "Rika321/%25",
            "Rika321/%FF",
            "Rika321/%F0%9F%A6%86",
            "Rika321/%CC%81a",
            "%E5%A4%A7/HorrorGame",
            "Rika321/大 黄",
        ] {
            assert_eq!(
                parse_repository_address(input, ORIGIN),
                Err(AddressError::Invalid),
                "{input:?}"
            );
        }
    }

    #[test]
    fn another_host_is_refused_before_any_request() {
        assert_eq!(
            parse_repository_address("https://staging.mallard-ai.com/Rika321/HorrorGame", ORIGIN),
            Err(AddressError::WrongService {
                host: "staging.mallard-ai.com".into(),
                expected: "mallard-ai.com".into()
            })
        );
        assert_eq!(
            parse_repository_address(
                "mallard-ai.com/Rika321/HorrorGame",
                "https://staging.mallard-ai.com"
            ),
            Err(AddressError::WrongService {
                host: "mallard-ai.com".into(),
                expected: "staging.mallard-ai.com".into()
            })
        );
        assert_eq!(
            parse_repository_address("localhost:9999/a/b", "http://localhost:8787"),
            Err(AddressError::WrongService {
                host: "localhost:9999".into(),
                expected: "localhost:8787".into()
            })
        );
        for input in [
            "https://github.com/Rika321/HorrorGame",
            "github.com/Rika321/HorrorGame.git",
            "https://www.github.com/Rika321/HorrorGame",
        ] {
            assert_eq!(
                parse_repository_address(input, ORIGIN),
                Err(AddressError::GitHub),
                "{input}"
            );
        }
    }

    #[test]
    fn what_is_not_an_address_is_invalid() {
        for input in [
            "",
            "   ",
            "HorrorGame",
            "https://mallard-ai.com/",
            "https://mallard-ai.com/Rika321",
            "https://mallard-ai.com/invitations/abc",
            "super-admin/HorrorGame",
            "https://mallard-ai.com/docs/push-and-pull",
            "docs/push-and-pull",
            "https://mallard-ai.com/repositories/HorrorGame",
            "mallard-ai.com/reviews/HorrorGame",
            "Privacy/HorrorGame",
            "ftp://mallard-ai.com/Rika321/HorrorGame",
            "https://user@mallard-ai.com/Rika321/HorrorGame",
            "Rika321/Horror Game",
            "Rika321/..",
            "git@github.com:Rika321/HorrorGame.git",
        ] {
            assert_eq!(
                parse_repository_address(input, ORIGIN),
                Err(AddressError::Invalid),
                "{input:?}"
            );
        }
        assert_eq!(
            parse_repository_address("Rika321/HorrorGame", "not an origin"),
            address("Rika321", "HorrorGame"),
            "a bare address needs no host"
        );
    }
}

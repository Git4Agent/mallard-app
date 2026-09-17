//! A Mallard Cloud repository address as a person pastes it: the page
//! address from the website (`https://mallard-ai.com/Rika321/HorrorGame`,
//! with or without the scheme, with or without a `/sessions/…` tail), or
//! the bare `owner/name`. The storage's own origin is the only host
//! accepted, so a staging address never reaches the production service.
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

/// The website's own top-level paths, which are never an owner login.
const RESERVED_OWNERS: [&str; 6] = [
    "invitations",
    "super-admin",
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

/// GitHub's repository character set, as the service's `isRepositoryName`.
fn name_shaped(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}

/// The host (with its port when the origin carries one) an address must
/// name to be the storage's, lower-cased; `www.` is dropped on both sides.
fn origin_host(service_origin: &str) -> Option<String> {
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

/// Parse `input` against the storage's `service_origin`.
pub fn parse_repository_address(
    input: &str,
    service_origin: &str,
) -> Result<RepositoryAddress, AddressError> {
    let text = input.trim();
    if text.is_empty() || text.len() > 2048 || text.chars().any(char::is_whitespace) {
        return Err(AddressError::Invalid);
    }
    // Drop a query or fragment: the website's session page carries `?commit=`.
    let text = text.split(['?', '#']).next().unwrap_or_default();
    let (host, path) = match text.split_once("://") {
        Some((scheme, rest)) => {
            if !(scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http")) {
                return Err(AddressError::Invalid);
            }
            let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
            (Some(authority), path)
        }
        None => {
            let (first, rest) = text.split_once('/').unwrap_or((text, ""));
            if looks_like_host(first) {
                (Some(first), rest)
            } else {
                (None, text)
            }
        }
    };
    if let Some(authority) = host {
        if authority.contains('@') {
            return Err(AddressError::Invalid);
        }
        let host = authority.to_ascii_lowercase();
        let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
        if host == "github.com" {
            return Err(AddressError::GitHub);
        }
        let expected = origin_host(service_origin).ok_or(AddressError::Invalid)?;
        if host != expected {
            return Err(AddressError::WrongService { host, expected });
        }
    }
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    let (Some(owner), Some(name)) = (segments.next(), segments.next()) else {
        return Err(AddressError::Invalid);
    };
    if !login_shaped(owner)
        || !name_shaped(name)
        || RESERVED_OWNERS
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(owner))
    {
        return Err(AddressError::Invalid);
    }
    Ok(RepositoryAddress {
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
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

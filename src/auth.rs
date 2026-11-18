//! Sign in with Aero. Identity only, and not OAuth: Aero's consent screen
//! sends the browser back to `/auth/aero` with a PASETO `v4.local` token,
//! the visitor's email encrypted with a key only Aero and this app hold. The
//! browser keeps it as an opaque string and sends it as a bearer token; the
//! API decrypts it on every call that needs to know who is asking.
//!
//! The same module as Champt's, which signs in the same way.
//!
//! Nothing here may log a token's contents or an email.

use pasetors::claims::ClaimsValidationRules;
use pasetors::keys::SymmetricKey;
use pasetors::token::UntrustedToken;
use pasetors::{Local, local, version4::V4};

/// The app's registration on Aero gives a client id and a key together, and
/// registering again replaces both, so both live in the backend's
/// environment. The key is a PASERK `k4.local.…` string and secret; the id
/// is public, but it still lives only here: the page starts a sign-in
/// through `/api/sign-in`, which adds it.
pub const CLIENT_ID_VAR: &str = "AERO_CLIENT_ID";
pub const KEY_VAR: &str = "AERO_LOGIN_KEY";

/// Aero's consent screen, where a sign-in starts.
const AERO_LOGIN_URL: &str = "https://aero.oops.wtf/login-with-aero";

/// How long a sign-in lasts: 30 days. The token is the session, and Aero
/// can't recall one before it runs out.
const SESSION_TTL_SECONDS: u64 = 30 * 24 * 60 * 60;

/// Someone signed in.
pub struct Person {
    /// Lowercased. Stamped on each report for audits; never sent to anyone.
    pub email: String,
}

pub struct Verifier {
    /// None when the key is missing or unreadable: then nobody can sign in.
    key: Option<SymmetricKey<V4>>,
    /// None when missing or not base64url: the same.
    client_id: Option<String>,
}

impl Verifier {
    /// Checks tokens encrypted with the PASERK `key` (`k4.local.…`) and
    /// addressed to `client_id`.
    pub fn new(key: &str, client_id: &str) -> Result<Verifier, pasetors::errors::Error> {
        let key = SymmetricKey::<V4>::try_from(key)?;
        Ok(Verifier { key: Some(key), client_id: usable(client_id) })
    }

    /// The real thing, from the environment. Without both values the API
    /// still answers, and says once why nobody can sign in.
    pub fn aero() -> Verifier {
        let key = std::env::var(KEY_VAR).ok().and_then(|text| SymmetricKey::<V4>::try_from(text.trim()).ok());
        let client_id = std::env::var(CLIENT_ID_VAR).ok().and_then(|text| usable(&text));
        if key.is_none() {
            eprintln!("{KEY_VAR} is missing or not a k4.local key; sign-ins will be refused");
        }
        if client_id.is_none() {
            eprintln!("{CLIENT_ID_VAR} is missing or malformed; sign-ins will be refused");
        }
        Verifier { key, client_id }
    }

    /// Aero's consent screen for a new sign-in, carrying the page's `state`
    /// back with the answer. None until both the client id and the key are
    /// set: a sign-in this app couldn't open would only end in a 401.
    /// `state` must already be URL-safe.
    pub fn sign_in_url(&self, state: &str) -> Option<String> {
        self.key.as_ref()?;
        let client_id = self.client_id.as_deref()?;
        Some(format!("{AERO_LOGIN_URL}?client_id={client_id}&state={state}&ttl={SESSION_TTL_SECONDS}"))
    }

    /// Who a token names, if it was made with this app's key, is addressed to
    /// this app and is currently valid. A token that decrypts with the key was
    /// made by Aero (or by this app), so there is no signature to check besides.
    pub fn person(&self, token: &str) -> Option<Person> {
        let key = self.key.as_ref()?;
        let client_id = self.client_id.as_deref()?;
        // `iat`, `nbf` and `exp` are checked by default, and `exp` is required.
        let mut rules = ClaimsValidationRules::new();
        rules.validate_audience_with(client_id);
        let untrusted = UntrustedToken::<Local, V4>::try_from(token).ok()?;
        let trusted = local::decrypt(key, &untrusted, &rules, None, None).ok()?;
        let email = trusted.payload_claims()?.get_claim("email")?.as_str()?.trim().to_lowercase();
        if email.is_empty() {
            return None;
        }
        Some(Person { email })
    }
}

/// A client id as Aero issues them: base64url. Anything else is treated as
/// missing, since it would break the sign-in link.
fn usable(client_id: &str) -> Option<String> {
    let id = client_id.trim();
    let fits = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    fits.then(|| id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pasetors::claims::Claims;
    use pasetors::keys::Generate;
    use pasetors::paserk::FormatAsPaserk;

    fn paserk(key: &SymmetricKey<V4>) -> String {
        let mut text = String::new();
        key.fmt(&mut text).unwrap();
        text
    }

    fn token(key: &SymmetricKey<V4>, aud: &str, email: &str) -> String {
        let mut claims = Claims::new().unwrap();
        claims.audience(aud).unwrap();
        claims.add_additional("state", "s").unwrap();
        claims.add_additional("email", email).unwrap();
        local::encrypt(key, &claims, None, None).unwrap()
    }

    #[test]
    fn a_token_names_its_person_in_lower_case() {
        let key = SymmetricKey::<V4>::generate().unwrap();
        let verifier = Verifier::new(&paserk(&key), "bikeqr").unwrap();
        let ada = verifier.person(&token(&key, "bikeqr", " Ada@Example.com ")).unwrap();
        assert_eq!(ada.email, "ada@example.com");
    }

    #[test]
    fn tokens_for_another_app_or_key_are_refused() {
        let key = SymmetricKey::<V4>::generate().unwrap();
        let verifier = Verifier::new(&paserk(&key), "bikeqr").unwrap();
        assert!(verifier.person(&token(&key, "champt", "ada@example.com")).is_none());
        let other = SymmetricKey::<V4>::generate().unwrap();
        assert!(verifier.person(&token(&other, "bikeqr", "ada@example.com")).is_none());
        assert!(verifier.person("v4.local.garbage").is_none());
    }

    #[test]
    fn nothing_gets_in_before_the_app_is_registered() {
        // A good token, addressed to whatever id, is still refused while the
        // app has no client id of its own.
        let key = SymmetricKey::<V4>::generate().unwrap();
        let verifier = Verifier::new(&paserk(&key), "").unwrap();
        assert!(verifier.person(&token(&key, "bikeqr", "ada@example.com")).is_none());
    }

    #[test]
    fn a_sign_in_starts_at_aero_with_the_client_id_and_30_days() {
        let key = SymmetricKey::<V4>::generate().unwrap();
        let verifier = Verifier::new(&paserk(&key), " AbC-12_x ").unwrap();
        assert_eq!(
            verifier.sign_in_url("s-1").as_deref(),
            Some("https://aero.oops.wtf/login-with-aero?client_id=AbC-12_x&state=s-1&ttl=2592000")
        );
        for unusable in ["", "   ", "has space", "a&b=c"] {
            assert!(Verifier::new(&paserk(&key), unusable).unwrap().sign_in_url("s").is_none(), "{unusable:?}");
        }
    }

    #[test]
    fn a_key_must_be_a_k4_local_paserk() {
        assert!(Verifier::new("not-a-key", "bikeqr").is_err());
        assert!(Verifier::new("k4.public.AAAA", "bikeqr").is_err());
    }
}

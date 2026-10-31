//! Seul accès réseau de l'application. Le mode hors ligne est garanti ici, par
//! construction : sans autorisation, aucune requête ne part.

use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

const USER_AGENT: &str = concat!("Komago/", env!("CARGO_PKG_VERSION"), " (local library application)");
const TIMEOUT: Duration = Duration::from_secs(20);
pub const MAX_JSON_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct HostRule {
    pub host: &'static str,
    /// Autorise aussi les sous-domaines.
    pub subdomains: bool,
    pub min_interval: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    pub url: String,
    pub json_body: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub retry_after: Option<u64>,
    pub body: Vec<u8>,
}

pub trait Transport: Send + Sync {
    fn send(&self, request: &HttpRequest, max_bytes: u64) -> std::result::Result<HttpResponse, String>;
}

/// Le client n'est créé qu'à la première requête : un problème de TLS ne doit pas
/// empêcher l'application de démarrer, et hors ligne il n'existe jamais.
pub struct ReqwestTransport {
    client: OnceLock<std::result::Result<reqwest::blocking::Client, String>>,
}

impl ReqwestTransport {
    pub fn new() -> Self {
        Self { client: OnceLock::new() }
    }

    fn client(&self) -> std::result::Result<&reqwest::blocking::Client, String> {
        self.client
            .get_or_init(|| {
                reqwest::blocking::Client::builder()
                    .user_agent(USER_AGENT)
                    .timeout(TIMEOUT)
                    // Une redirection pourrait sortir de la liste des hôtes autorisés.
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

impl Transport for ReqwestTransport {
    fn send(&self, request: &HttpRequest, max_bytes: u64) -> std::result::Result<HttpResponse, String> {
        let client = self.client()?;
        let builder = match &request.json_body {
            Some(body) => client
                .post(&request.url)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .body(body.clone()),
            None => client.get(&request.url),
        };
        let response = builder.send().map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok());
        let mut body = Vec::new();
        response.take(max_bytes + 1).read_to_end(&mut body).map_err(|e| e.to_string())?;
        Ok(HttpResponse { status, retry_after, body })
    }
}

pub struct GuardedClient {
    transport: Box<dyn Transport>,
    rules: Vec<HostRule>,
    enabled: AtomicBool,
    next_slot: Mutex<HashMap<&'static str, Instant>>,
}

impl GuardedClient {
    pub fn new(transport: Box<dyn Transport>, rules: Vec<HostRule>, enabled: bool) -> Self {
        Self { transport, rules, enabled: AtomicBool::new(enabled), next_slot: Mutex::new(HashMap::new()) }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    pub fn get(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>> {
        self.send(HttpRequest { url: url.to_string(), json_body: None }, max_bytes)
    }

    pub fn post_json(&self, url: &str, body: String, max_bytes: u64) -> Result<Vec<u8>> {
        self.send(HttpRequest { url: url.to_string(), json_body: Some(body) }, max_bytes)
    }

    fn send(&self, request: HttpRequest, max_bytes: u64) -> Result<Vec<u8>> {
        if !self.enabled.load(Ordering::SeqCst) {
            return Err(Error::NetworkDisabled);
        }
        let rule = self.rule_for(&request.url)?;
        self.wait_for_slot(&rule);

        let response = self
            .transport
            .send(&request, max_bytes)
            .map_err(|e| Error::Network(format!("connexion impossible : {e}")))?;
        match response.status {
            200..=299 => {}
            429 => {
                let wait = response.retry_after.map(|s| format!(", réessayez dans {s} s")).unwrap_or_default();
                return Err(Error::Network(format!("limite de requêtes atteinte{wait}")));
            }
            status => return Err(Error::Network(format!("réponse inattendue du service ({status})"))),
        }
        if response.body.len() as u64 > max_bytes {
            return Err(Error::Network("réponse trop volumineuse".into()));
        }
        Ok(response.body)
    }

    fn rule_for(&self, url: &str) -> Result<HostRule> {
        let parsed = reqwest::Url::parse(url).map_err(|_| Error::Network("adresse invalide".into()))?;
        if parsed.scheme() != "https" {
            return Err(Error::Network("connexion non chiffrée refusée".into()));
        }
        let host = parsed.host_str().unwrap_or_default();
        self.rules
            .iter()
            .find(|rule| host == rule.host || (rule.subdomains && host.ends_with(&format!(".{}", rule.host))))
            .copied()
            .ok_or_else(|| Error::Network(format!("hôte non autorisé : {host}")))
    }

    /// Réserve un créneau par service, puis attend hors du verrou.
    fn wait_for_slot(&self, rule: &HostRule) {
        let now = Instant::now();
        let start = {
            let mut slots = self.next_slot.lock().unwrap_or_else(|e| e.into_inner());
            let slot = slots.entry(rule.host).or_insert(now);
            let start = (*slot).max(now);
            *slot = start + rule.min_interval;
            start
        };
        if start > now {
            std::thread::sleep(start - now);
        }
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Répond par ordre d'arrivée et garde la trace des requêtes.
    #[derive(Clone, Default)]
    pub struct FakeTransport {
        pub requests: Arc<Mutex<Vec<HttpRequest>>>,
        pub responses: Arc<Mutex<Vec<(u16, Vec<u8>)>>>,
    }

    impl FakeTransport {
        pub fn with(responses: Vec<(u16, Vec<u8>)>) -> Self {
            Self { responses: Arc::new(Mutex::new(responses)), ..Default::default() }
        }
    }

    impl Transport for FakeTransport {
        fn send(&self, request: &HttpRequest, _max: u64) -> std::result::Result<HttpResponse, String> {
            self.requests.lock().unwrap().push(request.clone());
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err("aucune réponse prévue".into());
            }
            let (status, body) = responses.remove(0);
            Ok(HttpResponse { status, retry_after: Some(30), body })
        }
    }

    pub fn rule(host: &'static str, subdomains: bool) -> HostRule {
        HostRule { host, subdomains, min_interval: Duration::ZERO }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    fn client(fake: &FakeTransport, enabled: bool) -> GuardedClient {
        GuardedClient::new(Box::new(fake.clone()), vec![rule("api.test", false), rule("cdn.test", true)], enabled)
    }

    #[test]
    fn nothing_leaves_while_the_network_is_disabled() {
        let fake = FakeTransport::with(vec![(200, b"{}".to_vec())]);
        let guarded = client(&fake, false);

        assert!(matches!(guarded.get("https://api.test/x", 100), Err(Error::NetworkDisabled)));
        assert!(fake.requests.lock().unwrap().is_empty());

        guarded.set_enabled(true);
        assert!(guarded.get("https://api.test/x", 100).is_ok());
        guarded.set_enabled(false);
        assert!(matches!(guarded.post_json("https://api.test/x", "{}".into(), 100), Err(Error::NetworkDisabled)));
        assert_eq!(fake.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn only_https_on_allowed_hosts() {
        let fake = FakeTransport::default();
        let guarded = client(&fake, true);
        for url in [
            "http://api.test/x",
            "https://evil.test/x",
            "https://api.test.evil.test/x",
            "https://sub.api.test/x",
            "https://notcdn.test/x",
            "file:///etc/passwd",
            "nonsense",
        ] {
            assert!(guarded.get(url, 100).is_err(), "{url}");
        }
        assert!(fake.requests.lock().unwrap().is_empty());
        // Le sous-domaine autorisé atteint bien le transport (qui n'a pas de réponse prévue).
        let _ = guarded.get("https://s4.cdn.test/x", 100);
        assert_eq!(fake.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn errors_and_oversized_bodies_are_refused() {
        let fake = FakeTransport::with(vec![(500, vec![]), (429, vec![]), (200, vec![0; 11])]);
        let guarded = client(&fake, true);
        assert!(guarded.get("https://api.test/x", 10).is_err());
        let limited = guarded.get("https://api.test/x", 10).unwrap_err().to_string();
        assert!(limited.contains("30 s"), "{limited}");
        assert!(guarded.get("https://api.test/x", 10).is_err());
    }

    #[test]
    fn requests_to_a_service_are_spaced() {
        let fake = FakeTransport::with(vec![(200, vec![]), (200, vec![]), (200, vec![])]);
        let spaced = HostRule { host: "api.test", subdomains: false, min_interval: Duration::from_millis(40) };
        let guarded = GuardedClient::new(Box::new(fake), vec![spaced], true);
        let start = Instant::now();
        for _ in 0..3 {
            guarded.get("https://api.test/x", 10).unwrap();
        }
        assert!(start.elapsed() >= Duration::from_millis(80));
    }
}

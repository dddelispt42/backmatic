use crate::error::{BackmaticError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PingResult {
    pub url: String,
    pub body: Option<String>,
}

pub trait HttpClient: Send + Sync {
    fn post_ping(&self, base_url: &str, uuid: &str, body: Option<&str>) -> Result<PingResult>;
    fn post_fail(&self, base_url: &str, uuid: &str, body: &str) -> Result<PingResult>;
}

pub struct RealHttpClient {
    client: reqwest::blocking::Client,
}

impl RealHttpClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("http client"),
        }
    }
}

impl Default for RealHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient for RealHttpClient {
    fn post_ping(&self, base_url: &str, uuid: &str, body: Option<&str>) -> Result<PingResult> {
        let url = format!("{}/ping/{}", base_url.trim_end_matches('/'), uuid);
        let mut req = self.client.post(&url);
        if let Some(b) = body {
            req = req.body(b.to_string());
        }
        let _resp = req.send().map_err(|e| BackmaticError::Other(anyhow::anyhow!(e)))?;
        Ok(PingResult {
            url,
            body: body.map(str::to_string),
        })
    }

    fn post_fail(&self, base_url: &str, uuid: &str, body: &str) -> Result<PingResult> {
        let url = format!("{}/ping/{}/fail", base_url.trim_end_matches('/'), uuid);
        self.client
            .post(&url)
            .body(body.to_string())
            .send()
            .map_err(|e| BackmaticError::Other(anyhow::anyhow!(e)))?;
        Ok(PingResult {
            url,
            body: Some(body.to_string()),
        })
    }
}

#[cfg(any(test, feature = "integration-tests"))]
#[derive(Default)]
pub struct MockHttpClient {
    pub pings: std::sync::Mutex<Vec<PingResult>>,
}

#[cfg(any(test, feature = "integration-tests"))]
impl MockHttpClient {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(any(test, feature = "integration-tests"))]
impl HttpClient for MockHttpClient {
    fn post_ping(&self, base_url: &str, uuid: &str, body: Option<&str>) -> Result<PingResult> {
        let result = PingResult {
            url: format!("{}/ping/{}", base_url, uuid),
            body: body.map(str::to_string),
        };
        self.pings.lock().unwrap().push(result.clone());
        Ok(result)
    }

    fn post_fail(&self, base_url: &str, uuid: &str, body: &str) -> Result<PingResult> {
        let result = PingResult {
            url: format!("{}/ping/{}/fail", base_url, uuid),
            body: Some(body.to_string()),
        };
        self.pings.lock().unwrap().push(result.clone());
        Ok(result)
    }
}

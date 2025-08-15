use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use tokio::fs;
use chrono::{DateTime, Utc};
use anyhow::{Result, Context};
use tracing::{info, warn, debug, error};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleCredentials {
    pub access_token: Option<String>,
    pub refresh_token: String,
    pub client_id: String,
    pub client_secret: String,
    pub project_id: Option<String>,
    #[serde(default)]
    pub expiry: Option<DateTime<Utc>>,
    pub scope: Option<String>,
    pub scopes: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialState {
    pub error_codes: Vec<u16>,
    pub disabled: bool,
    pub last_success: Option<DateTime<Utc>>,
    pub cd_until: Option<DateTime<Utc>>,
}

impl Default for CredentialState {
    fn default() -> Self {
        Self {
            error_codes: Vec::new(),
            disabled: false,
            last_success: None,
            cd_until: None,
        }
    }
}

pub struct CredentialManager {
    credentials_dir: PathBuf,
    state_file: PathBuf,
    credential_states: HashMap<String, CredentialState>,
    credential_files: Vec<PathBuf>,
    current_index: usize,
    calls_per_rotation: usize,
    call_count: usize,
    http_client: reqwest::Client,
}

impl CredentialManager {
    pub fn new(credentials_dir: impl AsRef<Path>, calls_per_rotation: usize) -> Self {
        let credentials_dir = credentials_dir.as_ref().to_path_buf();
        let state_file = credentials_dir.join("creds_state.toml");

        Self {
            credentials_dir,
            state_file,
            credential_states: HashMap::new(),
            credential_files: Vec::new(),
            current_index: 0,
            calls_per_rotation,
            call_count: 0,
            http_client: reqwest::Client::new(),
        }
    }

    pub async fn initialize(&mut self) -> Result<()> {
        // Create credentials directory if it doesn't exist
        fs::create_dir_all(&self.credentials_dir).await
            .context("Failed to create credentials directory")?;

        // Load credential states
        self.load_states().await?;
        
        // Discover credential files
        self.discover_credential_files().await?;
        
        // Clean up expired CD statuses
        self.cleanup_expired_cd_status();

        info!("Credential manager initialized with {} credential files", self.credential_files.len());
        Ok(())
    }

    async fn load_states(&mut self) -> Result<()> {
        if self.state_file.exists() {
            let content = fs::read_to_string(&self.state_file).await
                .context("Failed to read state file")?;
            
            let states: HashMap<String, CredentialState> = toml::from_str(&content)
                .context("Failed to parse state file")?;
            
            self.credential_states = states;
        }
        Ok(())
    }

    async fn save_states(&self) -> Result<()> {
        let content = toml::to_string(&self.credential_states)
            .context("Failed to serialize credential states")?;
        
        fs::write(&self.state_file, content).await
            .context("Failed to write state file")?;
        
        Ok(())
    }

    fn cleanup_expired_cd_status(&mut self) {
        let now = Utc::now();
        let today_8am = now.date_naive().and_hms_opt(8, 0, 0)
            .map(|dt| dt.and_utc())
            .unwrap_or(now);

        let cutoff_time = if now >= today_8am {
            today_8am
        } else {
            today_8am - chrono::Duration::days(1)
        };

        for (filename, state) in self.credential_states.iter_mut() {
            if let Some(cd_until) = state.cd_until {
                if cd_until <= cutoff_time {
                    state.cd_until = None;
                    info!("Cleared expired CD status for {}", filename);
                }
            }
        }
    }

    async fn discover_credential_files(&mut self) -> Result<()> {
        let mut files = Vec::new();
        
        let mut dir_entries = fs::read_dir(&self.credentials_dir).await
            .context("Failed to read credentials directory")?;
        
        while let Some(entry) = dir_entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                let filename = path.to_string_lossy().to_string();
                
                // Check if credential is available (not disabled and not in CD)
                if !self.is_credential_disabled(&filename) && !self.is_credential_in_cd(&filename) {
                    files.push(path);
                }
            }
        }
        
        files.sort();
        self.credential_files = files;
        
        if self.credential_files.is_empty() {
            warn!("No available credential files found in {}", self.credentials_dir.display());
        } else {
            info!("Found {} available credential files", self.credential_files.len());
        }
        
        Ok(())
    }

    fn get_credential_state(&mut self, filename: &str) -> &mut CredentialState {
        self.credential_states.entry(filename.to_string()).or_default()
    }

    fn is_credential_disabled(&self, filename: &str) -> bool {
        self.credential_states.get(filename)
            .map(|state| state.disabled)
            .unwrap_or(false)
    }

    fn is_credential_in_cd(&self, filename: &str) -> bool {
        if let Some(state) = self.credential_states.get(filename) {
            if let Some(cd_until) = state.cd_until {
                return Utc::now() < cd_until;
            }
        }
        false
    }

    pub async fn record_error(&mut self, filename: &str, status_code: u16) -> Result<()> {
        let state = self.get_credential_state(filename);
        
        if !state.error_codes.contains(&status_code) {
            state.error_codes.push(status_code);
        }

        // Set CD status for 429 errors (rate limiting)
        if status_code == 429 {
            let now = Utc::now();
            let tomorrow_8am = (now + chrono::Duration::days(1))
                .date_naive()
                .and_hms_opt(8, 0, 0)
                .map(|dt| dt.and_utc())
                .unwrap_or(now + chrono::Duration::days(1));
            
            state.cd_until = Some(tomorrow_8am);
            warn!("Set CD status for {} until {}", filename, tomorrow_8am);
        }

        self.save_states().await?;
        Ok(())
    }

    pub async fn record_success(&mut self, filename: &str) -> Result<()> {
        let state = self.get_credential_state(filename);
        state.error_codes.clear();
        state.last_success = Some(Utc::now());
        
        self.save_states().await?;
        Ok(())
    }

    pub async fn set_credential_disabled(&mut self, filename: &str, disabled: bool) -> Result<()> {
        let state = self.get_credential_state(filename);
        state.disabled = disabled;
        
        info!("Setting disabled={} for file: {}", disabled, filename);
        
        // Re-discover files if we enabled/disabled a credential
        self.discover_credential_files().await?;
        
        self.save_states().await?;
        Ok(())
    }

    pub fn get_credentials_status(&self) -> HashMap<String, CredentialState> {
        self.credential_states.clone()
    }

    pub async fn get_current_credentials(&mut self) -> Result<Option<(GoogleCredentials, Option<String>)>> {
        // Check if we need to rotate credentials
        if self.call_count >= self.calls_per_rotation && !self.credential_files.is_empty() {
            self.current_index = (self.current_index + 1) % self.credential_files.len();
            self.call_count = 0;
            info!("Rotated to credential index {}", self.current_index);
        }

        // Re-discover files if we have no available credentials
        if self.credential_files.is_empty() {
            self.discover_credential_files().await?;
        }

        if self.credential_files.is_empty() {
            return Ok(None);
        }

        let current_file = &self.credential_files[self.current_index];
        
        match self.load_credentials_from_file(current_file).await {
            Ok(Some((creds, project_id))) => {
                debug!("Using credentials from {}", current_file.display());
                Ok(Some((creds, project_id)))
            },
            Ok(None) => {
                // Try next file on failure
                if self.credential_files.len() > 1 {
                    self.current_index = (self.current_index + 1) % self.credential_files.len();
                    let next_file = &self.credential_files[self.current_index];
                    self.load_credentials_from_file(next_file).await
                } else {
                    Ok(None)
                }
            },
            Err(e) => {
                error!("Failed to load credentials from {}: {}", current_file.display(), e);
                Ok(None)
            }
        }
    }

    async fn load_credentials_from_file(&self, file_path: &Path) -> Result<Option<(GoogleCredentials, Option<String>)>> {
        let content = fs::read_to_string(file_path).await
            .with_context(|| format!("Failed to read credential file: {}", file_path.display()))?;
        
        let mut creds: GoogleCredentials = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse credential file: {}", file_path.display()))?;

        if creds.refresh_token.is_empty() {
            warn!("No refresh token in {}", file_path.display());
            return Ok(None);
        }

        // Handle different credential formats
        let value: serde_json::Value = serde_json::from_str(&content)?;
        if creds.access_token.is_none() {
            // Try to load access_token from token field
            if let Some(token) = value.get("token").and_then(|v| v.as_str()) {
                creds.access_token = Some(token.to_string());
            }
        } else if let Some(token) = value.get("token").and_then(|v| v.as_str()) {
            // Prefer token field if it exists
            creds.access_token = Some(token.to_string());
        }

        // Handle scopes
        if creds.scopes.is_none() {
            if let Some(scope) = &creds.scope {
                creds.scopes = Some(scope.split_whitespace().map(|s| s.to_string()).collect());
            }
        }

        let project_id = creds.project_id.clone();
        
        Ok(Some((creds, project_id)))
    }

    pub fn increment_call_count(&mut self) {
        self.call_count += 1;
        debug!("Call count incremented to {}/{}", self.call_count, self.calls_per_rotation);
    }

    pub fn get_current_file_path(&self) -> Option<&Path> {
        self.credential_files.get(self.current_index).map(|p| p.as_path())
    }

    pub fn credentials_dir(&self) -> &Path {
        &self.credentials_dir
    }

    pub async fn refresh_credentials(&self, creds: &mut GoogleCredentials) -> Result<()> {
        if creds.expiry.map(|exp| exp <= Utc::now()).unwrap_or(true) && !creds.refresh_token.is_empty() {
            let refresh_req = serde_json::json!({
                "grant_type": "refresh_token",
                "refresh_token": creds.refresh_token,
                "client_id": creds.client_id,
                "client_secret": creds.client_secret,
            });

            let response = self.http_client
                .post("https://oauth2.googleapis.com/token")
                .json(&refresh_req)
                .send()
                .await
                .context("Failed to send refresh request")?;

            if response.status().is_success() {
                let refresh_resp: serde_json::Value = response.json().await
                    .context("Failed to parse refresh response")?;

                if let Some(access_token) = refresh_resp.get("access_token").and_then(|v| v.as_str()) {
                    creds.access_token = Some(access_token.to_string());
                    
                    if let Some(expires_in) = refresh_resp.get("expires_in").and_then(|v| v.as_i64()) {
                        creds.expiry = Some(Utc::now() + chrono::Duration::seconds(expires_in));
                    }
                    
                    debug!("Successfully refreshed credentials");
                }
            } else {
                let error_text = response.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!("Failed to refresh credentials: {}", error_text));
            }
        }
        
        Ok(())
    }
}
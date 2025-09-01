use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, error, info, warn};

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
    #[serde(skip)]
    pub credential_id: Option<String>,
}

impl GoogleCredentials {
    pub fn get_credential_id(&self) -> String {
        self.credential_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CredentialState {
    pub error_codes: Vec<u16>,
    pub disabled: bool,
    pub last_success: Option<DateTime<Utc>>,
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
    max_retries: usize,
}

impl CredentialManager {
    pub fn new(
        credentials_dir: impl AsRef<Path>,
        calls_per_rotation: usize,
        max_retries: usize,
    ) -> Self {
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
            max_retries,
        }
    }

    pub async fn initialize(&mut self) -> Result<()> {
        // Create credentials directory if it doesn't exist
        fs::create_dir_all(&self.credentials_dir)
            .await
            .context("Failed to create credentials directory")?;

        // Load credential states
        self.load_states().await?;

        // Discover credential files
        self.discover_credential_files().await?;

        info!(
            "Credential manager initialized with {} credential files",
            self.credential_files.len()
        );
        Ok(())
    }

    async fn load_states(&mut self) -> Result<()> {
        if self.state_file.exists() {
            let content = fs::read_to_string(&self.state_file)
                .await
                .context("Failed to read state file")?;

            let states: HashMap<String, CredentialState> =
                toml::from_str(&content).context("Failed to parse state file")?;

            self.credential_states = states;
        }
        Ok(())
    }

    async fn save_states(&self) -> Result<()> {
        let content = toml::to_string(&self.credential_states)
            .context("Failed to serialize credential states")?;

        fs::write(&self.state_file, content)
            .await
            .context("Failed to write state file")?;

        Ok(())
    }

    async fn discover_credential_files(&mut self) -> Result<()> {
        let mut files = Vec::new();

        let mut dir_entries = fs::read_dir(&self.credentials_dir)
            .await
            .context("Failed to read credentials directory")?;

        while let Some(entry) = dir_entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                let filename = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("unknown.json")
                    .to_string();

                // Check if credential is available (not disabled)
                if !self.is_credential_disabled(&filename) {
                    files.push(path);
                }
            }
        }

        files.sort();
        self.credential_files = files;

        if self.credential_files.is_empty() {
            warn!(
                "No available credential files found in {}",
                self.credentials_dir.display()
            );
        } else {
            info!(
                "Found {} available credential files",
                self.credential_files.len()
            );
        }

        Ok(())
    }

    fn get_credential_state(&mut self, filename: &str) -> &mut CredentialState {
        self.credential_states
            .entry(filename.to_string())
            .or_default()
    }

    fn is_credential_disabled(&self, filename: &str) -> bool {
        self.credential_states
            .get(filename)
            .map(|state| state.disabled)
            .unwrap_or(false)
    }

    pub async fn record_error(&mut self, filename: &str, status_code: u16) -> Result<()> {
        let state = self.get_credential_state(filename);

        if !state.error_codes.contains(&status_code) {
            state.error_codes.push(status_code);
        }

        // Simply record the error without any cooldown

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

    pub async fn get_current_credentials(
        &mut self,
    ) -> Result<Option<(GoogleCredentials, Option<String>)>> {
        // Check if we need to rotate credentials
        if self.call_count >= self.calls_per_rotation && !self.credential_files.is_empty() {
            self.current_index = (self.current_index + 1) % self.credential_files.len();
            self.call_count = 0;
            debug!("Rotated to credential index {}", self.current_index);
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
            }
            Ok(None) => {
                // Try next file on failure
                if self.credential_files.len() > 1 {
                    self.current_index = (self.current_index + 1) % self.credential_files.len();
                    let next_file = &self.credential_files[self.current_index];
                    self.load_credentials_from_file(next_file).await
                } else {
                    Ok(None)
                }
            }
            Err(e) => {
                error!(
                    "Failed to load credentials from {}: {}",
                    current_file.display(),
                    e
                );
                Ok(None)
            }
        }
    }

    async fn load_credentials_from_file(
        &self,
        file_path: &Path,
    ) -> Result<Option<(GoogleCredentials, Option<String>)>> {
        let content = fs::read_to_string(file_path)
            .await
            .with_context(|| format!("Failed to read credential file: {}", file_path.display()))?;

        let mut creds: GoogleCredentials = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse credential file: {}", file_path.display()))?;

        if creds.refresh_token.is_empty() {
            warn!("No refresh token in {}", file_path.display());
            return Ok(None);
        }

        // Generate credential ID from file name
        let file_name = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        creds.credential_id = Some(file_name);

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
        if creds.scopes.is_none()
            && let Some(scope) = &creds.scope
        {
            creds.scopes = Some(scope.split_whitespace().map(|s| s.to_string()).collect());
        }

        let project_id = creds.project_id.clone();

        Ok(Some((creds, project_id)))
    }

    pub fn increment_call_count(&mut self) {
        self.call_count += 1;
        debug!(
            "Call count incremented to {}/{}",
            self.call_count, self.calls_per_rotation
        );
    }

    pub fn get_current_file_path(&self) -> Option<&Path> {
        self.credential_files
            .get(self.current_index)
            .map(|p| p.as_path())
    }

    pub fn get_current_file_name(&self) -> Option<String> {
        self.credential_files
            .get(self.current_index)
            .and_then(|p| p.file_name())
            .and_then(|name| name.to_str())
            .map(|s| s.to_string())
    }

    pub fn credentials_dir(&self) -> &Path {
        &self.credentials_dir
    }

    pub fn max_retries(&self) -> usize {
        self.max_retries
    }

    pub async fn refresh_credentials(&self, creds: &mut GoogleCredentials) -> Result<()> {
        if creds.expiry.map(|exp| exp <= Utc::now()).unwrap_or(true)
            && !creds.refresh_token.is_empty()
        {
            let refresh_req = serde_json::json!({
                "grant_type": "refresh_token",
                "refresh_token": creds.refresh_token,
                "client_id": creds.client_id,
                "client_secret": creds.client_secret,
            });

            let response = self
                .http_client
                .post("https://oauth2.googleapis.com/token")
                .json(&refresh_req)
                .send()
                .await
                .context("Failed to send refresh request")?;

            if response.status().is_success() {
                let refresh_resp: serde_json::Value = response
                    .json()
                    .await
                    .context("Failed to parse refresh response")?;

                if let Some(access_token) =
                    refresh_resp.get("access_token").and_then(|v| v.as_str())
                {
                    creds.access_token = Some(access_token.to_string());

                    if let Some(expires_in) =
                        refresh_resp.get("expires_in").and_then(|v| v.as_i64())
                    {
                        creds.expiry = Some(Utc::now() + chrono::Duration::seconds(expires_in));
                    }

                    debug!(
                        "Successfully refreshed credentials for {}",
                        creds.get_credential_id()
                    );
                }
            } else {
                let error_text = response.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!(
                    "Failed to refresh credentials for {}: {}",
                    creds.get_credential_id(),
                    error_text
                ));
            }
        }

        Ok(())
    }

    pub async fn get_credentials_with_retry(
        &mut self,
    ) -> Result<Option<(GoogleCredentials, Option<String>)>> {
        // Re-discover files if we have no available credentials
        if self.credential_files.is_empty() {
            self.discover_credential_files().await?;
            if self.credential_files.is_empty() {
                debug!("No credential files available");
                return Ok(None);
            }
        }

        // Try to get current credentials (with rotation logic)
        match self.get_current_credentials().await {
            Ok(Some(result)) => return Ok(Some(result)),
            Ok(None) => {
                // If current credentials are not available, try other credentials immediately
                debug!("Current credentials not available, trying other credentials");
            }
            Err(e) => {
                // If there's an error getting current credentials, try other credentials
                debug!("Error getting current credentials: {}, trying other credentials", e);
            }
        }

        // Try other credentials
        let mut tried_indices = std::collections::HashSet::new();
        // Don't mark current index as tried yet, we'll try it last if needed
        let start_index = self.current_index;
        let mut attempts = 0;

        while attempts < self.max_retries && tried_indices.len() < self.credential_files.len() {
            // Move to next credential file
            self.current_index = (self.current_index + 1) % self.credential_files.len();
            
            // If we've tried all other credentials and are back to start, break
            if self.current_index == start_index && !tried_indices.is_empty() {
                break;
            }
            
            tried_indices.insert(self.current_index);

            let current_file = &self.credential_files[self.current_index];

            match self.load_credentials_from_file(current_file).await {
                Ok(Some((creds, project_id))) => {
                    debug!(
                        "Successfully loaded credentials: {}",
                        creds.get_credential_id()
                    );
                    return Ok(Some((creds, project_id)));
                }
                Ok(None) => {
                    warn!(
                        "Failed to load credentials from {}, trying next",
                        current_file.display()
                    );
                }
                Err(e) => {
                    error!(
                        "Error loading credentials from {}: {}",
                        current_file.display(),
                        e
                    );
                }
            }

            attempts += 1;
        }

        // If we still haven't found valid credentials, try the original current index one more time
        if attempts < self.max_retries && !tried_indices.contains(&start_index) {
            let current_file = &self.credential_files[start_index];
            match self.load_credentials_from_file(current_file).await {
                Ok(Some((creds, project_id))) => {
                    debug!(
                        "Successfully loaded credentials from original index: {}",
                        creds.get_credential_id()
                    );
                    // Update current index to the successful one
                    self.current_index = start_index;
                    return Ok(Some((creds, project_id)));
                }
                Ok(None) => {
                    warn!(
                        "Failed to load credentials from original index {}",
                        current_file.display()
                    );
                }
                Err(e) => {
                    error!(
                        "Error loading credentials from original index {}: {}",
                        current_file.display(),
                        e
                    );
                }
            }
        }

        if tried_indices.len() >= self.credential_files.len() {
            warn!(
                "All {} credential files have been tried",
                self.credential_files.len()
            );
        } else {
            warn!("Reached maximum retry attempts ({})", self.max_retries);
        }

        Ok(None)
    }
}

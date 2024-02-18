use std::path::Path;
use anyhow::Result;
use serde::Deserialize;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

const AUTH_BASE: &str = "https://auth.docker.io/token?service=registry.docker.io&scope=repository:library";
const REGISTRY_BASE: &str = "https://registry.hub.docker.com/v2/library";

pub struct DockerRegistryClient<'a> {
    client: reqwest::Client,
    image: &'a str,
    tag: &'a str,
    token: Option<String>,
}

impl<'a> DockerRegistryClient<'a> {
    pub fn for_image(image: &'a str, tag: &'a str) -> Self {
        Self {
            client: reqwest::Client::new(),
            image,
            tag,
            token: None,
        }
    }

    async fn get_token(&self) -> Result<String> {
        let url = format!("{AUTH_BASE}/{}:pull", self.image);
        let resp = self.client.get(&url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await?;

        if resp.status().is_success() {
            let data: serde_json::Value = resp.json().await?;
            let token = data["token"].as_str().unwrap().to_string();
            Ok(token)
        } else {
            Err(anyhow::Error::msg(format!("Failed to get auth token. Received response {}", resp.status())))
        }
    }

    async fn ensure_token(&mut self) -> Result<()> {
        if self.token.is_some() {
            return Ok(());
        }

        self.token = Some(self.get_token().await?);
        Ok(())
    }

    pub async fn get_manifest(&mut self) -> Result<DockerManifest> {
        self.ensure_token().await?;

        let url = format!("{REGISTRY_BASE}/{}/manifests/{}", self.image, self.tag);
        let response = self.client.get(&url)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", self.token.as_ref().unwrap()))
            .header(reqwest::header::ACCEPT, "application/vnd.docker.distribution.manifest.v2+json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow::Error::msg(format!("Failed to retrieve manifest for {}:{} - Status: {}", self.image, self.tag, response.status())));
        }

        let manifest_json: serde_json::Value = response.json().await?;
        let manifest: DockerManifest = serde_json::from_value(manifest_json)?;

        Ok(manifest)
    }

    pub async fn download_layer(&mut self, dest: impl AsRef<Path>, blob_hash: &str, media_type: &str) -> Result<()> {
        self.ensure_token().await?;

        //print!("Downloading {}...", blob_hash);
        let url = format!("{REGISTRY_BASE}/{}/blobs/{}", self.image, blob_hash);
        let mut response = self.client.get(&url)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", self.token.as_ref().unwrap()))
            .header(reqwest::header::ACCEPT, media_type)
            .send()
            .await?;

        if !response.status().is_success() {
            //println!("failed!");
            return Err(anyhow::Error::msg(format!("Failed to download layer. Status: {}", response.status())));
        }

        let mut file = File::options().write(true).create(true).open(dest).await?;
        loop {
            let chunk = response.chunk().await?;
            if chunk.is_none() {
                break;
            }

            let chunk = chunk.unwrap();
            file.write_all(chunk.as_ref()).await?;
        }

        //println!("done");

        Ok(())
    }
}

#[allow(unused)]
#[derive(Debug, Deserialize)]
pub struct DockerManifest {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub config: DockerManifestConfig,
    pub layers: Vec<DockerManifestLayer>,
}

#[allow(unused)]
#[derive(Debug, Deserialize)]
pub struct DockerManifestConfig {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub size: u64,
    pub digest: String,
}

#[allow(unused)]
#[derive(Debug, Deserialize)]
pub struct DockerManifestLayer {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub size: u64,
    pub digest: String,
}
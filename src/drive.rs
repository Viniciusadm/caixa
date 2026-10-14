use std::fs::File;
use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, LOCATION};
use serde::{Deserialize, Serialize};
use serde_json::json;

const API: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_API: &str = "https://www.googleapis.com/upload/drive/v3";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const FILE_FIELDS: &str = "id,name,size,mimeType,createdTime,webViewLink,trashed";
const CHUNK_SIZE: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    #[serde(default, deserialize_with = "size_from_string")]
    pub size: Option<u64>,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub created_time: String,
    #[serde(default)]
    pub web_view_link: String,
    #[serde(default)]
    pub trashed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileList {
    files: Vec<DriveFile>,
    next_page_token: Option<String>,
}

pub struct Drive {
    http: Client,
    token: String,
}

impl Drive {
    pub fn new(http: Client, token: String) -> Self {
        Self { http, token }
    }

    pub fn ensure_folder(&self, folder_id: Option<&str>, name: &str) -> Result<String> {
        if let Some(id) = folder_id.filter(|id| !id.is_empty()) {
            match self.get(id) {
                Ok(folder) if !folder.trashed => return Ok(folder.id),
                Ok(_) => {}
                Err(error) if error.to_string().contains("404") => {}
                Err(error) => return Err(error),
            }
        }
        let response = self
            .http
            .post(format!("{API}/files"))
            .bearer_auth(&self.token)
            .query(&[("fields", FILE_FIELDS)])
            .json(&json!({ "name": name, "mimeType": FOLDER_MIME }))
            .send()?;
        Ok(check(response)?.json::<DriveFile>()?.id)
    }

    pub fn get(&self, id: &str) -> Result<DriveFile> {
        let response = self
            .http
            .get(format!("{API}/files/{id}"))
            .bearer_auth(&self.token)
            .query(&[("fields", FILE_FIELDS)])
            .send()?;
        Ok(check(response)?.json()?)
    }

    pub fn list(&self) -> Result<Vec<DriveFile>> {
        let mut files = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut request = self.http.get(format!("{API}/files")).bearer_auth(&self.token).query(&[
                ("q", format!("trashed = false and mimeType != '{FOLDER_MIME}'").as_str()),
                ("fields", &format!("nextPageToken,files({FILE_FIELDS})")),
                ("orderBy", "createdTime desc"),
                ("pageSize", "1000"),
            ]);
            if let Some(token) = &page_token {
                request = request.query(&[("pageToken", token)]);
            }
            let page: FileList = check(request.send()?)?.json()?;
            files.extend(page.files);
            match page.next_page_token {
                Some(token) => page_token = Some(token),
                None => return Ok(files),
            }
        }
    }

    pub fn upload(&self, path: &Path, folder_id: &str) -> Result<DriveFile> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .with_context(|| format!("nome de arquivo inválido: {}", path.display()))?;
        let mut file = File::open(path).with_context(|| format!("não consegui abrir {}", path.display()))?;
        let total = file.metadata()?.len();
        let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();

        let response = self
            .http
            .post(format!("{UPLOAD_API}/files"))
            .bearer_auth(&self.token)
            .query(&[("uploadType", "resumable"), ("fields", FILE_FIELDS)])
            .header("X-Upload-Content-Type", &mime)
            .header("X-Upload-Content-Length", total)
            .json(&json!({ "name": name, "parents": [folder_id] }))
            .send()?;
        let session = check(response)?
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .context("o Drive não devolveu a URL de upload")?
            .to_string();

        let progress = Progress::new(name, total);
        let mut buffer = vec![0; CHUNK_SIZE];
        let mut sent: u64 = 0;
        loop {
            let read = read_full(&mut file, &mut buffer)?;
            let range = if total == 0 {
                "bytes */0".to_string()
            } else {
                format!("bytes {sent}-{}/{total}", sent + read as u64 - 1)
            };
            let response = self
                .http
                .put(&session)
                .header(CONTENT_RANGE, range)
                .header(CONTENT_LENGTH, read)
                .body(buffer[..read].to_vec())
                .send()?;
            sent += read as u64;
            progress.update(sent);
            if response.status() == StatusCode::PERMANENT_REDIRECT {
                if read == 0 {
                    bail!("upload de {name} interrompido: arquivo mudou durante o envio");
                }
                continue;
            }
            progress.finish();
            return Ok(check(response)?.json()?);
        }
    }

    pub fn make_public(&self, id: &str) -> Result<()> {
        let response = self
            .http
            .post(format!("{API}/files/{id}/permissions"))
            .bearer_auth(&self.token)
            .json(&json!({ "role": "reader", "type": "anyone" }))
            .send()?;
        check(response)?;
        Ok(())
    }

    pub fn download(&self, file: &DriveFile, destination: &Path) -> Result<()> {
        let mut response = check(
            self.http
                .get(format!("{API}/files/{}", file.id))
                .bearer_auth(&self.token)
                .query(&[("alt", "media")])
                .send()?,
        )?;
        let mut output =
            File::create(destination).with_context(|| format!("não consegui criar {}", destination.display()))?;
        let progress = Progress::new(&file.name, file.size.unwrap_or(0));
        let mut buffer = vec![0; 256 * 1024];
        let mut received: u64 = 0;
        loop {
            let read = response.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
            received += read as u64;
            progress.update(received);
        }
        progress.finish();
        Ok(())
    }

    pub fn trash(&self, id: &str) -> Result<()> {
        let response = self
            .http
            .patch(format!("{API}/files/{id}"))
            .bearer_auth(&self.token)
            .json(&json!({ "trashed": true }))
            .send()?;
        check(response)?;
        Ok(())
    }
}

fn check(response: Response) -> Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| value["error"]["message"].as_str().map(str::to_string))
        .unwrap_or(body);
    if status == StatusCode::NOT_FOUND {
        bail!("404: arquivo não encontrado (o cx só enxerga o que ele mesmo enviou)");
    }
    bail!("Drive respondeu {status}: {message}")
}

fn read_full(file: &mut File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        let read = file.read(&mut buffer[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}

fn size_from_string<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    let value: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::String(text)) => text.parse().ok(),
        Some(serde_json::Value::Number(number)) => number.as_u64(),
        _ => None,
    })
}

struct Progress {
    name: String,
    total: u64,
    enabled: bool,
}

impl Progress {
    fn new(name: &str, total: u64) -> Self {
        Self {
            name: name.to_string(),
            total,
            enabled: io::stderr().is_terminal(),
        }
    }

    fn update(&self, done: u64) {
        if !self.enabled {
            return;
        }
        let detail = match (done * 100).checked_div(self.total) {
            Some(percent) => format!("{percent:>3}%"),
            None => human_size(done),
        };
        eprint!("\r\x1b[2K{} {detail}", self.name);
    }

    fn finish(&self) {
        if self.enabled {
            eprint!("\r\x1b[2K");
        }
    }
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(10 * 1024 * 1024), "10.0 MB");
    }

    #[test]
    fn parses_size_sent_as_string() {
        let file: DriveFile = serde_json::from_str(r#"{"id":"a","name":"b","size":"2048"}"#).unwrap();
        assert_eq!(file.size, Some(2048));
    }
}

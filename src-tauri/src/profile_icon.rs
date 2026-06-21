use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

use base64::Engine as _;
use image::{imageops, ImageFormat, ImageReader};
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json;
use tauri::{path::BaseDirectory, AppHandle, Manager};
use tracing::error;

#[derive(Deserialize)]
#[allow(unused)]
struct IdResponse {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct ProfileResponse {
    properties: Vec<ProfileProperty>,
}

#[derive(Deserialize)]
struct ProfileProperty {
    name: String,
    value: String,
}

#[derive(Deserialize)]
struct TexturesPayload {
    textures: Textures,
}

#[derive(Deserialize)]
struct Textures {
    #[serde(rename = "SKIN")]
    skin: Option<TextureUrl>,
}

#[derive(Deserialize)]
struct TextureUrl {
    url: String,
}

#[derive(Serialize, Deserialize, Default)]
struct IconCacheMeta {
    etag: Option<String>,
    skin_url: Option<String>,
}

fn sanitize_username(username: &str) -> String {
    username
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn cache_dir_path(app: &AppHandle) -> Result<PathBuf, StatusCode> {
    app.path()
        .resolve("profile-icons", BaseDirectory::Cache)
        .map_err(|e| {
            error!(error = %e, "Error resolving profile icon cache dir");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

fn cache_paths(app: &AppHandle, username: &str) -> Result<(PathBuf, PathBuf), StatusCode> {
    let cache_dir = cache_dir_path(app)?;
    fs::create_dir_all(&cache_dir).map_err(|e| {
        error!(error = %e, "Error creating profile icon cache dir");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let safe_name = sanitize_username(username);
    let icon_path = cache_dir.join(format!("{safe_name}.png"));
    let meta_path = cache_dir.join(format!("{safe_name}.json"));
    Ok((icon_path, meta_path))
}

fn read_cache_meta(path: &Path) -> Option<IconCacheMeta> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn write_cache_meta(path: &Path, meta: &IconCacheMeta) -> Result<(), StatusCode> {
    let content = serde_json::to_string(meta).map_err(|e| {
        error!(error = %e, "Error encoding profile icon cache metadata");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    fs::write(path, content).map_err(|e| {
        error!(error = %e, "Error writing profile icon cache metadata");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(())
}

fn png_to_data_url(png_bytes: &[u8]) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(png_bytes);
    format!("data:image/png;base64,{}", encoded)
}

async fn fetch_skin_url(username: &str) -> Result<String, StatusCode> {
    let client = Client::new();

    let response_id = client
        .get(format!(
            "https://api.minecraftservices.com/minecraft/profile/lookup/name/{}",
            username
        ))
        .send()
        .await
        .map_err(|e| {
            error!(error = %e, "Error retrieving UUID");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if !response_id.status().is_success() {
        error!(response = %response_id.status(), "Error retrieving UUID");
        return Err(response_id.status());
    }

    let player_uuid = response_id.json::<IdResponse>().await.map_err(|e| {
        error!(error = %e, "Error parsing UUID");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let response_profile = client
        .get(format!(
            "https://sessionserver.mojang.com/session/minecraft/profile/{}",
            &player_uuid.id
        ))
        .send()
        .await
        .map_err(|e| {
            error!(error = %e, "Error retrieving profile");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if !response_profile.status().is_success() {
        error!(response = %response_profile.status(), "Error retrieving profile");
        return Err(response_profile.status());
    }

    let player_profile = response_profile
        .json::<ProfileResponse>()
        .await
        .map_err(|e| {
            error!(error = %e, "Error parsing profile");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let textures_property = player_profile
        .properties
        .into_iter()
        .find(|property| property.name == "textures")
        .ok_or_else(|| {
            error!("Missing textures property");
            StatusCode::NOT_FOUND
        })?;

    let decoded_textures = base64::engine::general_purpose::STANDARD
        .decode(textures_property.value.as_bytes())
        .map_err(|e| {
            error!(error = %e, "Error decoding textures base64");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let textures_payload: TexturesPayload =
        serde_json::from_slice(&decoded_textures).map_err(|e| {
            error!(error = %e, "Error parsing textures JSON");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let skin_url = textures_payload
        .textures
        .skin
        .ok_or_else(|| {
            error!("Missing skin texture");
            StatusCode::NOT_FOUND
        })?
        .url;

    Ok(skin_url)
}

async fn process_icon_from_skin(skin: &[u8]) -> Result<Vec<u8>, StatusCode> {
    let skin_image = ImageReader::new(Cursor::new(skin))
        .with_guessed_format()
        .map_err(|e| {
            error!(error = %e, "Invalid skin image");
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        })?
        .decode()
        .map_err(|e| {
            error!(error = %e, "Invalid skin image");
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        })?;

    let mut head_cropped = imageops::crop_imm(&skin_image, 8, 8, 8, 8).to_image();
    let hat_cropped = imageops::crop_imm(&skin_image, 40, 8, 8, 8).to_image();
    imageops::overlay(&mut head_cropped, &hat_cropped, 0, 0);

    let mut png_bytes = Vec::new();
    {
        let mut cursor = Cursor::new(&mut png_bytes);
        image::DynamicImage::ImageRgba8(head_cropped)
            .write_to(&mut cursor, ImageFormat::Png)
            .map_err(|e| {
                error!(error = %e, "Error encoding skin icon");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }

    Ok(png_bytes)
}

pub async fn get_profile_icon(app: &AppHandle, username: &str) -> Result<String, StatusCode> {
    let (icon_path, meta_path) = cache_paths(app, username)?;
    let cached_png = fs::read(&icon_path).ok();
    let cached_meta = read_cache_meta(&meta_path).unwrap_or_default();

    let skin_url = if let Some(skin_url) = cached_meta.skin_url.clone() {
        skin_url
    } else {
        fetch_skin_url(username).await?
    };

    let etag = cached_meta
        .etag
        .filter(|_| cached_meta.skin_url.as_deref() == Some(skin_url.as_str()));

    let client = Client::new();
    let mut request = client.get(&skin_url);
    if let Some(etag) = etag.as_deref() {
        request = request.header(header::IF_NONE_MATCH, etag);
    }

    let response_skin = request.send().await.map_err(|e| {
        error!(error = %e, "Error retrieving skin image");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if response_skin.status() == StatusCode::NOT_MODIFIED {
        if let Some(png_bytes) = cached_png {
            return Ok(png_to_data_url(&png_bytes));
        }
    }

    let response_skin = if response_skin.status() == StatusCode::NOT_MODIFIED {
        client.get(&skin_url).send().await.map_err(|e| {
            error!(error = %e, "Error retrieving skin image");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
    } else {
        response_skin
    };

    if !response_skin.status().is_success() {
        error!(response = %response_skin.status(), "Error retrieving skin image");
        return Err(response_skin.status());
    }

    let etag = response_skin
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());

    let skin_bytes = response_skin.bytes().await.map_err(|e| {
        error!(error = %e, "Error reading skin image bytes");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let png_bytes = process_icon_from_skin(&skin_bytes).await?;
    fs::write(&icon_path, &png_bytes).map_err(|e| {
        error!(error = %e, "Error writing profile icon cache");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    write_cache_meta(
        &meta_path,
        &IconCacheMeta {
            etag,
            skin_url: Some(skin_url),
        },
    )?;

    Ok(png_to_data_url(&png_bytes))
}

pub fn reset_profile_icon_cache(app: &AppHandle) -> Result<(), StatusCode> {
    let cache_dir = cache_dir_path(app)?;
    if cache_dir.exists() {
        fs::remove_dir_all(&cache_dir).map_err(|e| {
            error!(error = %e, "Error clearing profile icon cache");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    Ok(())
}

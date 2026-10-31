use std::fs;
use std::io::Cursor;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, ImageReader, Limits};
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{Error, Result};

const MAX_DIMENSION: u32 = 30_000;
const MAX_ALLOC: u64 = 1024 * 1024 * 1024;
const COVER_WIDTH: u32 = 600;
const COVER_HEIGHT: u32 = 900;

pub struct Cover {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Réduit une page en couverture. Les dimensions sont vérifiées avant décodage,
/// et une image hostile ne peut au pire que produire une erreur.
pub fn render(page: &[u8]) -> Result<Cover> {
    let decode = || -> Result<DynamicImage> {
        let mut reader = ImageReader::new(Cursor::new(page))
            .with_guessed_format()
            .map_err(|e| Error::Invalid(format!("image illisible : {e}")))?;
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_DIMENSION);
        limits.max_image_height = Some(MAX_DIMENSION);
        limits.max_alloc = Some(MAX_ALLOC);
        reader.limits(limits);
        reader.decode().map_err(|e| Error::Invalid(format!("image illisible : {e}")))
    };
    let image = catch_unwind(AssertUnwindSafe(decode))
        .map_err(|_| Error::Invalid("image illisible".into()))??;

    // Une page de webtoon est une longue bande : la couverture en montre le haut.
    let image = if image.height() > image.width().saturating_mul(2) {
        image.crop_imm(0, 0, image.width(), image.width().saturating_mul(3) / 2)
    } else {
        image
    };
    let thumbnail = image.resize(COVER_WIDTH, COVER_HEIGHT, FilterType::Triangle).into_rgb8();
    let (width, height) = thumbnail.dimensions();
    let mut bytes = Cursor::new(Vec::new());
    thumbnail
        .write_to(&mut bytes, ImageFormat::Jpeg)
        .map_err(|e| Error::Invalid(format!("couverture impossible à créer : {e}")))?;
    Ok(Cover { bytes: bytes.into_inner(), width, height })
}

/// Enregistre la couverture sur disque, puis en base. Le nom est l'empreinte du
/// contenu : deux séries de même couverture partagent le même fichier.
pub fn store(connection: &Connection, covers_dir: &Path, cover: &Cover) -> Result<String> {
    let hash = blake3::hash(&cover.bytes).to_hex().to_string();
    let file_name = format!("{hash}.jpg");
    let relative = format!("covers/{file_name}");
    let existing: Option<String> = connection
        .query_row("SELECT id FROM assets WHERE path = ?1", [&relative], |row| row.get(0))
        .optional()?;
    fs::write(covers_dir.join(&file_name), &cover.bytes)?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id = uuid::Uuid::now_v7().to_string();
    connection.execute(
        "INSERT INTO assets (id, kind, path, content_hash, width, height) VALUES (?1, 'cover', ?2, ?3, ?4, ?5)",
        params![id, relative, hash, cover.width, cover.height],
    )?;
    Ok(id)
}

#[cfg(test)]
pub mod test_support {
    use image::{ImageFormat, Rgb, RgbImage};
    use std::io::Cursor;

    pub fn png(width: u32, height: u32) -> Vec<u8> {
        let image = RgbImage::from_pixel(width, height, Rgb([200, 40, 40]));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::png;
    use super::*;

    #[test]
    fn large_pages_are_reduced() {
        let cover = render(&png(1400, 2000)).unwrap();
        assert!(cover.width <= COVER_WIDTH && cover.height <= COVER_HEIGHT);
        assert_eq!(&cover.bytes[..3], [0xFF, 0xD8, 0xFF]);
    }

    #[test]
    fn webtoon_strips_keep_only_the_top() {
        let cover = render(&png(800, 12000)).unwrap();
        assert!(cover.height > cover.width);
        assert!(cover.height <= cover.width * 2);
    }

    #[test]
    fn invalid_images_return_an_error() {
        assert!(render(&[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3]).is_err());
        assert!(render(b"nothing").is_err());
    }
}

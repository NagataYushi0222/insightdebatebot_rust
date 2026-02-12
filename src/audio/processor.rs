//! Audio processor for format conversion
//!
//! Handles conversion between audio formats using ffmpeg for Gemini API

use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, error, info, warn};

#[derive(Error, Debug)]
pub enum ProcessorError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("File not found: {0}")]
    NotFound(PathBuf),
    #[error("FFmpeg conversion failed: {0}")]
    ConversionFailed(String),
}

/// Audio processor for format operations
pub struct AudioProcessor;

impl AudioProcessor {
    /// Get MIME type for audio file
    pub fn get_mime_type(path: &Path) -> &'static str {
        match path.extension().and_then(|e| e.to_str()) {
            Some("ogg") => "audio/ogg",
            Some("opus") => "audio/ogg",
            Some("mp3") => "audio/mp3",
            Some("wav") => "audio/wav",
            Some("flac") => "audio/flac",
            Some("pcm") => "audio/pcm",
            _ => "audio/ogg",
        }
    }

    /// Convert an OGG/Opus file to MP3 using ffmpeg
    ///
    /// Returns the path to the converted MP3 file
    pub async fn convert_to_mp3(input_path: &Path) -> Result<PathBuf, ProcessorError> {
        if !input_path.exists() {
            return Err(ProcessorError::NotFound(input_path.to_path_buf()));
        }

        let mp3_path = input_path.with_extension("mp3");

        info!("Converting {:?} to MP3...", input_path);

        let output = tokio::process::Command::new("ffmpeg")
            .args([
                "-y",                    // Overwrite output
                "-i",                    // Input file
                input_path.to_str().unwrap_or(""),
                "-ac", "1",              // Mono (reduce size)
                "-ar", "16000",          // 16kHz sample rate (sufficient for speech)
                "-b:a", "64k",           // 64kbps bitrate
                "-f", "mp3",             // Output format
                mp3_path.to_str().unwrap_or(""),
            ])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("FFmpeg conversion failed: {}", stderr);
            return Err(ProcessorError::ConversionFailed(stderr.to_string()));
        }

        info!("Converted to MP3: {:?}", mp3_path);

        // Remove original OGG file
        if let Err(e) = fs::remove_file(input_path) {
            warn!("Failed to remove original file {:?}: {}", input_path, e);
        }

        Ok(mp3_path)
    }

    /// Clean up temporary audio files
    pub fn cleanup_files(paths: &[PathBuf]) {
        for path in paths {
            if path.exists() {
                match fs::remove_file(path) {
                    Ok(_) => debug!("Removed temp file: {:?}", path),
                    Err(e) => warn!("Failed to remove {:?}: {}", path, e),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mime_types() {
        assert_eq!(AudioProcessor::get_mime_type(Path::new("test.ogg")), "audio/ogg");
        assert_eq!(AudioProcessor::get_mime_type(Path::new("test.mp3")), "audio/mp3");
    }
}


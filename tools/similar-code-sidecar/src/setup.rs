use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use crate::cache::{
    ModelPaths, copy_and_hash, inspect_cache, remove_partial, replace_file, write_manifest,
};
use crate::constants::{ARTIFACTS, ArtifactSpec, MODEL_HUB_URL, MODEL_REVISION};

pub trait ArtifactDownloader {
    fn open(&self, url: &str) -> Result<Box<dyn Read>, String>;
}

pub struct HttpDownloader;

impl ArtifactDownloader for HttpDownloader {
    fn open(&self, url: &str) -> Result<Box<dyn Read>, String> {
        let response = ureq::get(url)
            .header(
                "User-Agent",
                concat!("fallow-similar-code/", env!("CARGO_PKG_VERSION")),
            )
            .call()
            .map_err(|error| format!("model download failed: {error}"))?;
        Ok(Box::new(response.into_body().into_reader()))
    }
}

#[derive(Clone, Debug)]
pub struct SetupResult {
    pub downloaded: bool,
}

pub fn install(paths: &ModelPaths) -> Result<SetupResult, String> {
    install_with(paths, ARTIFACTS, &HttpDownloader)
}

pub fn install_with(
    paths: &ModelPaths,
    artifacts: &[ArtifactSpec],
    downloader: &dyn ArtifactDownloader,
) -> Result<SetupResult, String> {
    if artifacts == ARTIFACTS && inspect_cache(paths, true).ready {
        return Ok(SetupResult { downloaded: false });
    }

    fs::create_dir_all(&paths.directory)
        .map_err(|error| format!("failed to create the model cache: {error}"))?;
    for artifact in artifacts {
        install_artifact(paths, *artifact, downloader)?;
    }
    write_manifest(&paths.manifest)?;
    Ok(SetupResult { downloaded: true })
}

fn install_artifact(
    paths: &ModelPaths,
    artifact: ArtifactSpec,
    downloader: &dyn ArtifactDownloader,
) -> Result<(), String> {
    let destination = paths
        .artifact(artifact.path)
        .ok_or_else(|| format!("unsupported model artifact `{}`", artifact.path))?;
    let partial = destination.with_extension("download-partial");
    remove_partial(&partial);
    let result = download_artifact(&partial, artifact, downloader);
    if let Err(error) = result {
        remove_partial(&partial);
        return Err(error);
    }
    replace_file(&partial, destination)
        .map_err(|error| format!("failed to install `{}`: {error}", artifact.path))
}

fn download_artifact(
    partial: &Path,
    artifact: ArtifactSpec,
    downloader: &dyn ArtifactDownloader,
) -> Result<(), String> {
    let url = format!("{MODEL_HUB_URL}/resolve/{MODEL_REVISION}/{}", artifact.path);
    let reader = downloader.open(&url)?;
    let mut reader = reader.take(artifact.size.saturating_add(1));
    let mut file = File::create(partial)
        .map_err(|error| format!("failed to create `{}`: {error}", artifact.path))?;
    let (size, sha256) = copy_and_hash(&mut reader, &mut file)?;
    if size != artifact.size {
        return Err(format!(
            "downloaded `{}` has the wrong size: expected {}, received {size}",
            artifact.path, artifact.size
        ));
    }
    if sha256 != artifact.sha256 {
        return Err(format!(
            "downloaded `{}` failed SHA-256 verification",
            artifact.path
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        reason = "test fixture construction must fail immediately"
    )]

    use std::collections::BTreeMap;
    use std::io::Cursor;

    use sha2::{Digest, Sha256};

    use super::*;

    struct MemoryDownloader {
        files: BTreeMap<String, Vec<u8>>,
    }

    impl ArtifactDownloader for MemoryDownloader {
        fn open(&self, url: &str) -> Result<Box<dyn Read>, String> {
            let name = url.rsplit('/').next().unwrap_or_default();
            self.files
                .get(name)
                .cloned()
                .map(|bytes| Box::new(Cursor::new(bytes)) as Box<dyn Read>)
                .ok_or_else(|| "missing test artifact".to_string())
        }
    }

    fn spec(name: &'static str, bytes: &[u8]) -> ArtifactSpec {
        let sha256 = crate::cache::digest_hex(Sha256::digest(bytes));
        ArtifactSpec {
            path: name,
            size: bytes.len() as u64,
            sha256: Box::leak(sha256.into_boxed_str()),
        }
    }

    #[test]
    fn setup_verifies_test_artifacts_without_network() {
        let directory = tempfile::tempdir().expect("tempdir");
        let paths = ModelPaths::from_cache_root(directory.path());
        let files = BTreeMap::from([
            ("model.safetensors".to_string(), b"model".to_vec()),
            ("tokenizer.json".to_string(), b"tokenizer".to_vec()),
            ("config.json".to_string(), b"config".to_vec()),
        ]);
        let artifacts = [
            spec("model.safetensors", &files["model.safetensors"]),
            spec("tokenizer.json", &files["tokenizer.json"]),
            spec("config.json", &files["config.json"]),
        ];

        let result =
            install_with(&paths, &artifacts, &MemoryDownloader { files }).expect("offline setup");
        assert!(result.downloaded);
        for artifact in artifacts {
            assert!(paths.artifact(artifact.path).is_some_and(Path::is_file));
        }
    }

    #[test]
    fn setup_rejects_a_digest_mismatch_and_removes_partial_file() {
        let directory = tempfile::tempdir().expect("tempdir");
        let paths = ModelPaths::from_cache_root(directory.path());
        let artifacts = [ArtifactSpec {
            path: "config.json",
            size: 3,
            sha256: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        }];
        let downloader = MemoryDownloader {
            files: BTreeMap::from([("config.json".to_string(), b"bad".to_vec())]),
        };

        let error = install_with(&paths, &artifacts, &downloader).expect_err("digest mismatch");
        assert!(error.contains("SHA-256"));
        assert!(!paths.config.with_extension("download-partial").exists());
    }
}

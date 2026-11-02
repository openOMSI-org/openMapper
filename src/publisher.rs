//! Explicit publisher authoring for signed and optional protected packs.

use serde::Deserialize;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::AtomicBool;
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Specification {
    #[serde(rename = "protected", default)]
    encrypt: bool,
    metadata: omsi_pack::PackMetadata,
    assets: Vec<Input>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    id: String,
    path: String,
    file: PathBuf,
    source: PathBuf,
}

fn read(path: &Path, limit: u64) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        !omsi_cfg::vfs::is_sealed(path),
        "protected playback content cannot be used as an authoring source"
    );
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "authoring input must be a regular file"
    );
    anyhow::ensure!(
        metadata.len() <= limit,
        "authoring input exceeds its size limit"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= limit,
        "authoring input exceeds its size limit"
    );
    Ok(bytes)
}

fn input_path(base: &Path, relative: &Path) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        !relative.is_absolute()
            && relative
                .components()
                .all(|p| matches!(p, Component::Normal(_))),
        "asset source paths must stay inside the specification folder"
    );
    let path = base.join(relative).canonicalize()?;
    anyhow::ensure!(
        path.starts_with(base),
        "asset source escaped the specification folder"
    );
    Ok(path)
}

/// Publisher-only operation: all plaintext inputs are explicit owned authoring files.
pub fn publish(
    specification: &Path,
    destination: &Path,
    signing_file: &Path,
    key_file: Option<&Path>,
) -> anyhow::Result<omsi_pack::Manifest> {
    anyhow::ensure!(
        !destination.exists(),
        "refusing to overwrite an existing pack"
    );
    let mut specification_data: Specification =
        serde_json::from_slice(&read(specification, 1024 * 1024)?)?;
    anyhow::ensure!(
        !specification_data.assets.is_empty() && specification_data.assets.len() <= 4096,
        "pack asset count exceeds its limit"
    );
    anyhow::ensure!(
        specification_data.metadata.key_id.is_none(),
        "the producer assigns a fresh content key and key ID"
    );
    anyhow::ensure!(
        specification_data.encrypt == key_file.is_some(),
        "protected publication needs a new private content-key output file"
    );
    let base = specification
        .parent()
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let signing_path = signing_file.canonicalize()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        anyhow::ensure!(
            std::fs::metadata(&signing_path)?.permissions().mode() & 0o077 == 0,
            "publisher signing key must be owner-only"
        );
    }
    let signing_bytes = Zeroizing::new(read(&signing_path, 16 * 1024)?);
    let signer = omsi_pack::SigningKey::from_pkcs8(&signing_bytes)?;
    let mut inputs = Vec::new();
    let mut total = 0u64;
    for input in specification_data.assets {
        let file = input_path(&base, &input.file)?;
        let source = input_path(&base, &input.source)?;
        anyhow::ensure!(
            file != signing_path && source != signing_path,
            "publisher private keys cannot be packaged as assets"
        );
        let bytes = read(&file, 64 * 1024 * 1024)?;
        total = total
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("pack size overflow"))?;
        anyhow::ensure!(total <= 1024 * 1024 * 1024, "pack exceeds 1 GiB");
        // Hash original bytes independently of the converted playable output.
        let source_sha256 = crate::provenance::file_hash(&source)?;
        inputs.push(omsi_pack::AssetInput {
            id: input.id,
            path: input.path,
            source_sha256,
            bytes,
        });
    }
    let key = if specification_data.encrypt {
        Some(Zeroizing::new(omsi_pack::random_content_key()?))
    } else {
        None
    };
    if let Some(key) = &key {
        specification_data.metadata.key_id = Some(omsi_pack::sha256_hex(&**key));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let output = key_file.ok_or_else(|| anyhow::anyhow!("content key output missing"))?;
        let mut file = options.open(output)?;
        if let Err(error) = file.write_all(&**key).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = std::fs::remove_file(output);
            return Err(error.into());
        }
    }
    let result = omsi_pack::create_pack(
        destination,
        specification_data.metadata,
        &inputs,
        &signer,
        key.as_ref().map(|key| &**key),
        &AtomicBool::new(false),
    );
    if result.is_err() {
        if let Some(path) = key_file {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(result?)
}

pub fn cli(
    specification: &Path,
    destination: &Path,
    signing_file: &Path,
    key_file: Option<&Path>,
) -> i32 {
    match publish(specification, destination, signing_file, key_file) {
        Ok(manifest) => {
            println!(
                "Published {} {} ({} assets)",
                manifest.pack_id,
                manifest.version,
                manifest.assets.len()
            );
            0
        }
        Err(error) => {
            eprintln!("publisher: {error}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    #[test]
    fn publisher_output_authenticates_through_the_actual_mesh_loader_and_uses_fresh_keys() {
        let root =
            std::env::temp_dir().join(format!("openmapper-publisher-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let original = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        std::fs::write(root.join("source.obj"), original).unwrap();
        let texel = [23, 97, 181, 255];
        image::RgbaImage::from_pixel(16, 16, image::Rgba(texel))
            .save(root.join("wall.png"))
            .unwrap();
        let content = root.join("content");
        std::fs::create_dir_all(&content).unwrap();
        let imported = crate::import::import(&root.join("source.obj"), &content).unwrap();
        let roots = crate::install::Roots {
            omsi: Default::default(),
            content: content.clone(),
        };
        let (object, _) = crate::editable::folder_of(&roots, &imported.objects[0]);
        let converted = std::fs::read_dir(object.join("model"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "x"))
            .unwrap();
        let converted = converted
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let signer = omsi_pack::SigningKey::generate(&ring::rand::SystemRandom::new()).unwrap();
        let signing = root.join("signing.pk8");
        signer.save_private_key(&signing).unwrap();
        let spec = root.join("spec.json");
        std::fs::write(&spec, serde_json::to_vec(&serde_json::json!({
            "protected": true,
            "metadata": {"publisher_id":"owned-fixture","author":"Fixture","pack_id":"mesh","version":"1.0.0","license":"LicenseRef-Owned-Fixture","redistribution":false,"key_id":null,"dependencies":[]},
            "assets":[
                {"id":"mesh","path":"Sceneryobjects/Fixture/model/triangle.x","file":converted,"source":"source.obj"},
                {"id":"texture","path":"Sceneryobjects/Fixture/texture/wall.png","file":"wall.png","source":"wall.png"}
            ]
        })).unwrap()).unwrap();
        let out = root.join("mesh.oopp");
        let key_path = root.join("content-key.bin");
        let manifest = super::publish(&spec, &out, &signing, Some(&key_path)).unwrap();
        assert_eq!(
            manifest.assets[0].source_sha256,
            omsi_pack::sha256_hex(original)
        );
        assert_ne!(
            manifest.assets[0].source_sha256,
            manifest.assets[0].output_sha256
        );
        let key: [u8; 32] = std::fs::read(&key_path).unwrap().try_into().unwrap();
        let mut trust = omsi_pack::TrustedPublishers::default();
        trust.insert("owned-fixture", signer.key_id(), signer.public_key());
        let pack =
            Arc::new(omsi_pack::VerifiedPack::open_for_authoring(&out, &trust, Some(key)).unwrap());
        let mount = root.join("virtual");
        omsi_cfg::vfs::mount_publisher(&mount, pack, BTreeMap::new()).unwrap();
        let mesh_path = mount.join("Sceneryobjects/Fixture/model/triangle.x");
        assert!(omsi_cfg::vfs::is_sealed(&mesh_path));
        let decoded = omsi_o3d::load_mesh(&mesh_path).unwrap();
        assert_eq!(decoded.triangles.len(), 1);
        assert!(!mesh_path.exists(), "protected asset was extracted to disk");
        let texture_path = mount.join("Sceneryobjects/Fixture/texture/wall.png");
        let decoded_texture = omsi_texture::decode_file(&texture_path).unwrap();
        assert_eq!((decoded_texture.width, decoded_texture.height), (16, 16));
        assert_eq!(&decoded_texture.rgba[..4], &texel[..]);
        let cache_dir = root.join("texture-cache");
        let cache = omsi_texture::disk_cache::DiskCache::new(cache_dir.clone(), 1024 * 1024);
        let bytes = omsi_cfg::vfs::read(&texture_path).unwrap();
        let options = omsi_texture::gpu::GpuOptions {
            bc: true,
            compress: true,
        };
        let _ = omsi_texture::gpu::load_gpu_cached(&bytes, &texture_path, options, Some(&cache))
            .unwrap();
        assert!(
            !cache_dir.exists(),
            "a protected texture was written to the official disk cache"
        );
        assert!(
            !texture_path.exists(),
            "a protected texture was extracted to disk"
        );
        let second = super::publish(
            &spec,
            &root.join("second.oopp"),
            &signing,
            Some(&root.join("second-key.bin")),
        )
        .unwrap();
        assert_ne!(manifest.key_id, second.key_id);
        assert!(super::publish(&spec, &out, &signing, Some(&key_path)).is_err());
        omsi_cfg::vfs::unmount_sealed(&mount);
        std::fs::remove_dir_all(root).unwrap();
    }
}

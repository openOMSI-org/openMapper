//! Explicit publisher authoring for signed and optional protected packs.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
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

fn load_specification(specification: &Path) -> anyhow::Result<Specification> {
    let specification_data: Specification =
        serde_json::from_slice(&read(specification, 1024 * 1024)?)?;
    anyhow::ensure!(
        !specification_data.assets.is_empty() && specification_data.assets.len() <= 4096,
        "pack asset count exceeds its limit"
    );
    anyhow::ensure!(
        specification_data.metadata.key_id.is_none(),
        "the producer assigns a fresh content key and key ID"
    );
    Ok(specification_data)
}

/// Read the signing key and every authoring input named by the specification.
fn load_inputs(
    specification: &Path,
    specification_data: Specification,
    signing_file: &Path,
) -> anyhow::Result<(
    omsi_pack::PackMetadata,
    Vec<omsi_pack::AssetInput>,
    omsi_pack::SigningKey,
)> {
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
    Ok((specification_data.metadata, inputs, signer))
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
    let specification_data = load_specification(specification)?;
    let encrypt = specification_data.encrypt;
    anyhow::ensure!(
        encrypt == key_file.is_some(),
        "protected publication needs a new private content-key output file"
    );
    let (mut metadata, inputs, signer) =
        load_inputs(specification, specification_data, signing_file)?;
    let key = if encrypt {
        Some(Zeroizing::new(omsi_pack::random_content_key()?))
    } else {
        None
    };
    if let Some(key) = &key {
        metadata.key_id = Some(omsi_pack::sha256_hex(&**key));
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
        metadata,
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

/// One sale's pack, as listed in the publisher's private `licences.json` record.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LicenceRecord {
    pub licence_id: String,
    pub pack_id: String,
    pub version: String,
    pub manifest_sha256: String,
    pub key_id: String,
    pub expires_unix: u64,
}

/// Licence IDs use the engine's format, which is also the placement-watermark buyer ID format.
fn licence_ids(bytes: &[u8]) -> anyhow::Result<Vec<String>> {
    let mut seen = BTreeSet::new();
    let mut ids = Vec::new();
    for id in std::str::from_utf8(bytes)?
        .lines()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        anyhow::ensure!(
            id.len() <= 128
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')),
            "licence IDs must be 1 to 128 ASCII letters, digits, dots, underscores or hyphens"
        );
        // Output file names are per ID, and some file systems ignore case.
        anyhow::ensure!(
            seen.insert(id.to_ascii_lowercase()),
            "the licence list repeats a licence ID"
        );
        ids.push(id.to_owned());
    }
    anyhow::ensure!(
        !ids.is_empty() && ids.len() <= 10_000,
        "the licence list must name 1 to 10,000 licence IDs"
    );
    Ok(ids)
}

fn write_private(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// Publisher-only batch: one protected pack, content key and licence for each opaque licence
/// ID. The pack and licence go to that buyer; the key and `licences.json` stay private.
pub fn publish_licensed(
    specification: &Path,
    output: &Path,
    signing_file: &Path,
    licences: &Path,
    expires_unix: u64,
) -> anyhow::Result<Vec<LicenceRecord>> {
    anyhow::ensure!(
        !output.exists(),
        "refusing to overwrite an existing output folder"
    );
    let ids = licence_ids(&read(licences, 1024 * 1024)?)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    anyhow::ensure!(expires_unix > now, "licence expiry must be in the future");
    let specification_data = load_specification(specification)?;
    anyhow::ensure!(
        specification_data.encrypt,
        "per-sale packs must be protected"
    );
    let (metadata, inputs, signer) = load_inputs(specification, specification_data, signing_file)?;
    #[cfg(unix)]
    let folder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        builder
    };
    #[cfg(not(unix))]
    let folder = std::fs::DirBuilder::new();
    folder.create(output)?;
    let result = publish_sales(output, &metadata, &inputs, &signer, &ids, expires_unix);
    if result.is_err() {
        // This call created the folder, so nothing else is removed.
        let _ = std::fs::remove_dir_all(output);
    }
    result
}

fn publish_sales(
    output: &Path,
    metadata: &omsi_pack::PackMetadata,
    inputs: &[omsi_pack::AssetInput],
    signer: &omsi_pack::SigningKey,
    ids: &[String],
    expires_unix: u64,
) -> anyhow::Result<Vec<LicenceRecord>> {
    let mut records = Vec::with_capacity(ids.len());
    for id in ids {
        let key = Zeroizing::new(omsi_pack::random_content_key()?);
        let key_id = omsi_pack::sha256_hex(&*key);
        write_private(&output.join(format!("{id}.key")), &*key)?;
        let mut sale = metadata.clone();
        sale.key_id = Some(key_id.clone());
        let manifest = omsi_pack::create_licensed_pack(
            &output.join(format!("{id}.oopp")),
            sale,
            id,
            inputs,
            signer,
            &key,
            &AtomicBool::new(false),
        )?;
        let licence = Zeroizing::new(omsi_pack::issue_entitlement(
            &manifest,
            *key,
            expires_unix,
            signer,
        )?);
        write_private(&output.join(format!("{id}.licence.json")), &licence)?;
        records.push(LicenceRecord {
            licence_id: id.clone(),
            pack_id: manifest.pack_id.clone(),
            version: manifest.version.clone(),
            manifest_sha256: omsi_pack::sha256_hex(&serde_json::to_vec(&manifest)?),
            key_id,
            expires_unix,
        });
    }
    write_private(
        &output.join("licences.json"),
        &serde_json::to_vec_pretty(&records)?,
    )?;
    Ok(records)
}

pub fn licensed_cli(
    specification: &Path,
    output: &Path,
    signing_file: &Path,
    licences: &Path,
    expires: &std::ffi::OsStr,
) -> i32 {
    let Some(expires_unix) = expires.to_str().and_then(|e| e.parse::<u64>().ok()) else {
        eprintln!("publisher: licence expiry must be Unix seconds");
        return 2;
    };
    match publish_licensed(specification, output, signing_file, licences, expires_unix) {
        Ok(records) => {
            println!(
                "Published {} per-sale packs and licences into {}. Send each buyer only their \
                 .oopp folder and .licence.json file; keep the .key files and licences.json private.",
                records.len(),
                output.display()
            );
            0
        }
        Err(error) => {
            eprintln!("publisher: {error}");
            1
        }
    }
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

    #[derive(Default)]
    struct MemoryKeys(std::sync::Mutex<BTreeMap<String, Vec<u8>>>);

    impl omsi_pack::KeyStore for MemoryKeys {
        fn store(&self, scope: &str, bytes: &[u8]) -> omsi_pack::Result<()> {
            self.0.lock().unwrap().insert(scope.into(), bytes.to_vec());
            Ok(())
        }
        fn load(&self, scope: &str) -> omsi_pack::Result<Vec<u8>> {
            let keys = self.0.lock().unwrap();
            keys.get(scope)
                .cloned()
                .ok_or(omsi_pack::PackError::KeyNotFound)
        }
        fn delete(&self, scope: &str) -> omsi_pack::Result<()> {
            let mut keys = self.0.lock().unwrap();
            keys.remove(scope)
                .map(|_| ())
                .ok_or(omsi_pack::PackError::KeyNotFound)
        }
    }

    #[test]
    fn licensed_publication_gives_each_sale_its_own_pack_key_and_licence() {
        let root = std::env::temp_dir().join(format!(
            "openmapper-licensed-publisher-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let config = b"[name]\r\nFixture\r\n";
        std::fs::write(root.join("global.cfg"), config).unwrap();
        let signer = omsi_pack::SigningKey::generate(&ring::rand::SystemRandom::new()).unwrap();
        let signing = root.join("signing.pk8");
        signer.save_private_key(&signing).unwrap();
        let write_spec = |name: &str, protected: bool| {
            let path = root.join(name);
            std::fs::write(&path, serde_json::to_vec(&serde_json::json!({
                "protected": protected,
                "metadata": {"publisher_id":"owned-fixture","author":"Fixture","pack_id":"map","version":"1.0.0","license":"LicenseRef-Owned-Fixture","redistribution":false,"key_id":null,"dependencies":[]},
                "assets":[{"id":"config","path":"maps/Fixture/global.cfg","file":"global.cfg","source":"global.cfg"}]
            })).unwrap()).unwrap();
            path
        };
        let spec = write_spec("spec.json", true);
        let ids = root.join("licences.txt");
        std::fs::write(&ids, "lic-a\n\nlic-b\n").unwrap();
        let expires = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let out = root.join("sales");
        let records = super::publish_licensed(&spec, &out, &signing, &ids, expires).unwrap();
        assert_eq!(
            records
                .iter()
                .map(|r| r.licence_id.as_str())
                .collect::<Vec<_>>(),
            ["lic-a", "lic-b"]
        );
        assert_ne!(records[0].key_id, records[1].key_id);
        let written: Vec<super::LicenceRecord> =
            serde_json::from_slice(&std::fs::read(out.join("licences.json")).unwrap()).unwrap();
        assert_eq!(written, records);

        let mut trust = omsi_pack::TrustedPublishers::default();
        trust.insert("owned-fixture", signer.key_id(), signer.public_key());
        let keys = MemoryKeys::default();
        let pack = |id: &str| out.join(format!("{id}.oopp"));
        let licence = |id: &str| std::fs::read(out.join(format!("{id}.licence.json"))).unwrap();
        let mut sources = Vec::new();
        for record in &records {
            let id = record.licence_id.as_str();
            let manifest = omsi_pack::load_manifest(&pack(id), &trust).unwrap();
            assert_eq!(manifest.licence_id.as_deref(), Some(id));
            assert_eq!(manifest.key_id.as_ref(), Some(&record.key_id));
            assert_eq!(
                omsi_pack::sha256_hex(&std::fs::read(pack(id).join("manifest.json")).unwrap()),
                record.manifest_sha256
            );
            let key = std::fs::read(out.join(format!("{id}.key"))).unwrap();
            assert_eq!(omsi_pack::sha256_hex(&key), record.key_id);
            #[cfg(unix)]
            for private in [format!("{id}.key"), format!("{id}.licence.json")] {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(out.join(private))
                    .unwrap()
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o077, 0, "per-sale secrets must be owner-only");
            }
            sources.push(manifest.assets[0].source_sha256.clone());
        }
        assert_eq!(sources[0], omsi_pack::sha256_hex(config));
        assert_eq!(
            sources[0], sources[1],
            "provenance stays the same for every sale"
        );
        assert!(omsi_pack::unlock(&pack("lic-b"), &licence("lic-a"), &trust, &keys).is_err());
        assert!(omsi_pack::unlock(&pack("lic-a"), &licence("lic-b"), &trust, &keys).is_err());
        for id in ["lic-a", "lic-b"] {
            omsi_pack::unlock(&pack(id), &licence(id), &trust, &keys).unwrap();
            let opened = omsi_pack::VerifiedPack::open_unlocked(&pack(id), &trust, &keys).unwrap();
            assert_eq!(opened.read("maps/Fixture/global.cfg").unwrap(), config);
        }

        let refused = root.join("refused");
        let unprotected = write_spec("plain.json", false);
        assert!(super::publish_licensed(&unprotected, &refused, &signing, &ids, expires).is_err());
        for list in ["lic-a\nlic-a\n", "has space\n", "\n", "lic-\u{fc}\n"] {
            std::fs::write(&ids, list).unwrap();
            assert!(
                super::publish_licensed(&spec, &refused, &signing, &ids, expires).is_err(),
                "accepted licence list {list:?}"
            );
        }
        std::fs::write(&ids, "lic-c\n").unwrap();
        assert!(super::publish_licensed(&spec, &refused, &signing, &ids, 1).is_err());
        assert!(!refused.exists(), "a refused batch left output behind");
        assert!(super::publish_licensed(&spec, &out, &signing, &ids, expires).is_err());
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 7);
        std::fs::remove_dir_all(root).unwrap();
    }
}

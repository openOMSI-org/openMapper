//! Trusted publisher provenance and ordinary export policy.

use omsi_cfg::publisher::{AssetReference, Catalog, MapReferences};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Policy {
    pub references: MapReferences,
    pub warnings: Vec<String>,
    pub unresolved: Vec<String>,
    pub rights_available: bool,
}

/// A rights declaration changes ordinary redistribution policy, never protected playback.
pub fn filter(
    files: &mut BTreeMap<String, PathBuf>,
    content: &Path,
    rights: bool,
) -> anyhow::Result<Policy> {
    for (relative, path) in files.iter() {
        if omsi_cfg::vfs::is_sealed(path) {
            anyhow::bail!("protected playback assets cannot be exported as plaintext: {relative}");
        }
    }
    let catalog = Catalog::load(content)?;
    if catalog.entries.is_empty() { return Ok(Policy::default()); }
    let history = crate::provenance::History::load(content)?;
    let mut by_hash: BTreeMap<&str, Vec<_>> = BTreeMap::new();
    for entry in &catalog.entries {
        by_hash.entry(&entry.source_sha256).or_default().push(entry);
        if entry.output_sha256 != entry.source_sha256 {
            by_hash.entry(&entry.output_sha256).or_default().push(entry);
        }
    }
    let mut policy = Policy::default();
    let mut omitted = Vec::new();
    for (relative, path) in files.iter() {
        if omsi_cfg::vfs::is_sealed(path) {
            anyhow::bail!("protected playback assets cannot be exported as plaintext: {relative}");
        }
        // Only the map's used files are read; the signed library index contains the hashes.
        let hash = crate::provenance::file_hash(path)?;
        let mut matches = by_hash.get(hash.as_str()).cloned().unwrap_or_default();
        if let Some(originals) = history.entries.get(&hash) {
            for original in originals {
                matches.extend(by_hash.get(original.as_str()).cloned().unwrap_or_default());
            }
        }
        let mut claims = BTreeSet::new();
        matches.retain(|a| {
            claims.insert((
                a.dependency.publisher_id.clone(),
                a.dependency.pack_id.clone(),
                a.dependency.version.clone(),
                a.asset_id.clone(),
            ))
        });
        if matches.is_empty() {
            continue;
        }
        for claim in &matches {
            policy.warnings.push(format!(
                "{relative}: publisher {}/{}/{}; author {}; licence {}; redistribution {}",
                claim.dependency.publisher_id,
                claim.dependency.pack_id,
                claim.dependency.version,
                claim.author,
                claim.license,
                if claim.redistribution {
                    "permitted"
                } else {
                    "restricted"
                }
            ));
        }
        if matches.iter().all(|a| a.redistribution && !a.protected) {
            continue;
        }
        policy.rights_available |= matches.iter().all(|claim| !claim.protected);
        if rights && matches.iter().all(|claim| !claim.protected) {
            policy.warnings.push(format!(
                "{relative}: included under the exporter's explicit rights declaration"
            ));
            continue;
        }
        // Exact playable bytes give an unambiguous alias, even after the file was renamed.
        matches.sort_by_key(|a| {
            (
                a.redistribution && !a.protected,
                a.dependency.publisher_id.clone(),
                a.dependency.pack_id.clone(),
                a.asset_id.clone(),
            )
        });
        let exact = matches.iter().find(|a| a.output_sha256 == hash
            && omsi_cfg::publisher::validate_asset_alias(relative, &a.path).is_ok());
        omitted.push(relative.clone());
        if let Some(asset) = exact {
            if !policy.references.dependencies.contains(&asset.dependency) {
                policy
                    .references
                    .dependencies
                    .push(asset.dependency.clone());
            }
            policy.references.assets.push(AssetReference {
                path: relative.clone(),
                dependency: asset.dependency.clone(),
                asset_id: asset.asset_id.clone(),
            });
        } else if relative.ends_with(".openmapper.json") {
            policy.warnings.push(format!(
                "{relative}: editable source omitted; the published pack supplies playback assets"
            ));
        } else if relative.to_ascii_lowercase().ends_with(".sco") {
            // An authored object wrapper is an overlay of mesh references, not the mesh itself.
            omitted.pop();
        } else {
            let remedy = if matches.iter().any(|claim| claim.protected) {
                "obtain the publisher's playable asset; protected content has no plaintext export override"
            } else {
                "obtain its converted asset or declare your redistribution rights"
            };
            policy.unresolved.push(format!("{relative}: no byte-identical playable output in the trusted publisher pack; {remedy}"));
        }
    }
    for relative in omitted {
        files.remove(&relative);
    }
    Ok(policy)
}

#[cfg(test)]
mod tests {
    use omsi_pack::{AssetInput, PackMetadata, SigningKey, TrustedPublishers};
    use std::collections::BTreeMap;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn renamed_restricted_asset_exports_a_dependency_and_keeps_the_ordinary_rights_override() {
        let root = std::env::temp_dir().join(format!("om-publisher-policy-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Packs")).unwrap();
        let signer = SigningKey::generate(&ring::rand::SystemRandom::new()).unwrap();
        let mut trust = TrustedPublishers::default();
        trust.insert("owned-fixture", signer.key_id(), signer.public_key());
        trust.save_file(&root.join("Publishers.json")).unwrap();
        let data = b"[friendlyname]\nOwned sign\n";
        let pack = root.join("Packs/signs.oopp");
        omsi_pack::create_pack(
            &pack,
            PackMetadata {
                publisher_id: "owned-fixture".into(),
                author: "Fixture".into(),
                pack_id: "signs".into(),
                version: "1.0.0".into(),
                license: "LicenseRef-Owned-Fixture".into(),
                redistribution: false,
                key_id: None,
                dependencies: vec![],
            },
            &[AssetInput {
                id: "sign".into(),
                path: "Sceneryobjects/Fixture/sign.sco".into(),
                source_sha256: omsi_pack::sha256_hex(b"original fixture source"),
                bytes: data.to_vec(),
            }],
            &signer,
            None,
            &AtomicBool::new(false),
        )
        .unwrap();
        let renamed = root.join("renamed.sco");
        std::fs::write(&renamed, data).unwrap();
        let alias = "Sceneryobjects/MyMap/renamed.sco".to_string();
        let mut files = BTreeMap::from([(alias.clone(), renamed.clone())]);
        let policy = super::filter(&mut files, &root, false).unwrap();
        assert!(files.is_empty(), "restricted bytes were copied");
        assert_eq!(policy.references.dependencies[0].pack_id, "signs");
        assert_eq!(policy.references.assets[0].path, alias);
        assert_eq!(policy.references.assets[0].asset_id, "sign");
        assert!(!policy.warnings.is_empty());
        assert!(policy.rights_available);

        // Retaining the trusted index still recognizes a renamed copy without its manifest.
        std::fs::remove_file(pack.join("manifest.json")).unwrap();
        let mut referenced = BTreeMap::from([(alias.clone(), renamed.clone())]);
        assert!(
            !super::filter(&mut referenced, &root, false)
                .unwrap()
                .references
                .assets
                .is_empty()
        );
        let mut overridden = BTreeMap::from([(alias.clone(), renamed.clone())]);
        let declared = super::filter(&mut overridden, &root, true).unwrap();
        assert_eq!(overridden.len(), 1);
        assert!(declared.references.dependencies.is_empty());

        // A changed output invalidates the exact hash match immediately.
        std::fs::write(&renamed, b"different owned source bytes").unwrap();
        let mut changed = BTreeMap::from([(alias, renamed)]);
        assert!(
            super::filter(&mut changed, &root, false)
                .unwrap()
                .references
                .assets
                .is_empty()
        );
        assert_eq!(changed.len(), 1);
        let key = omsi_pack::random_content_key().unwrap();
        let protected = b"owned protected playable bytes";
        omsi_pack::create_pack(&root.join("Packs/protected.oopp"), PackMetadata {
            publisher_id: "owned-fixture".into(), author: "Fixture".into(), pack_id: "protected".into(),
            version: "1.0.0".into(), license: "LicenseRef-Owned-Fixture".into(), redistribution: false,
            key_id: Some(omsi_pack::sha256_hex(&key)), dependencies: vec![],
        }, &[AssetInput { id: "mesh".into(), path: "Sceneryobjects/Fixture/model/mesh.x".into(),
            source_sha256: omsi_pack::sha256_hex(protected), bytes: protected.to_vec() }],
            &signer, Some(&key), &AtomicBool::new(false)).unwrap();
        let physical = root.join("owned-clear.x");
        std::fs::write(&physical, protected).unwrap();
        let mut files = BTreeMap::from([("Sceneryobjects/Fixture/model/owned.x".into(), physical)]);
        let declared = super::filter(&mut files, &root, true).unwrap();
        assert!(files.is_empty(), "ordinary rights declaration bypassed protected-pack policy");
        assert_eq!(declared.references.assets[0].asset_id, "mesh");
        assert!(!declared.rights_available);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_ordinary_rights_declaration_cannot_export_a_protected_virtual_asset() {
        let root = std::env::temp_dir().join(format!("om-protected-export-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let key = std::sync::Arc::new(omsi_cfg::vfs::SealKey::new(
            &omsi_cfg::vfs::random_key().unwrap(),
        ));
        let blob = root.join("blob");
        std::fs::write(
            &blob,
            omsi_cfg::vfs::seal(&key, b"fixture", b"owned bytes").unwrap(),
        )
        .unwrap();
        let mount = root.join("virtual");
        omsi_cfg::vfs::mount_sealed(
            &mount,
            key,
            vec![omsi_cfg::vfs::SealedFile {
                rel: "Sceneryobjects/Fixture/a.sco".into(),
                blob,
                aad: b"fixture".to_vec(),
                size: 11,
            }],
        );
        let mut files = BTreeMap::from([(
            "Sceneryobjects/Fixture/a.sco".into(),
            mount.join("Sceneryobjects/Fixture/a.sco"),
        )]);
        assert!(super::filter(&mut files, &root, true).is_err());
        omsi_cfg::vfs::unmount_sealed(&mount);
        std::fs::remove_dir_all(root).unwrap();
    }
}

use super::*;

static EXPORT_ZIP_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A map with an imported model (and its texture and PBR map) and a stock bus stop,
/// exported and imported into a fresh content folder: it opens with every file there, the
/// model's own files packed and the installation's left out.
#[test]
fn an_exported_map_imports_whole() {
    let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else { return };
    let _guard = EXPORT_ZIP_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let _map = crate::doc::one_map();
    let base = std::env::temp_dir().join(format!("openmapper-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (c1, c2, src) = (base.join("content1"), base.join("content2"), base.join("model"));
    for d in [&c1, &c2, &src] {
        std::fs::create_dir_all(d).unwrap();
    }
    // a model the way a beginner brings one in
    std::fs::write(src.join("kiosk.mtl"), "newmtl a\nKd 1 1 1\nmap_Kd kiosk.png\n").unwrap();
    std::fs::write(src.join("kiosk.obj"), "mtllib kiosk.mtl\nv 0 0 0\nv 2 0 0\nv 2 2 0\nv 0 2 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl a\nf 1/1 2/2 3/3 4/4\n").unwrap();
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 100, 50, 255]));
    img.save(src.join("kiosk.png")).unwrap();
    img.save(src.join("kiosk_nn.png")).unwrap();
    let r = crate::import::import(&src.join("kiosk.obj"), &c1).unwrap();
    let kiosk = r.objects[0].clone();

    let roots = Roots { omsi: omsi.clone(), content: c1.clone() };
    let mut doc = Document::new_map(&roots, "ExportTest", "Export test", 1);
    doc.global.description = "A map to share.".into();
    let t = doc.tiles.get_mut(&(0, 0)).unwrap();
    for (id, file) in [(1, kiosk.clone()), (2, "Sceneryobjects\\Generic\\bus_stop.sco".to_string())] {
        t.tile.objects.push(omsi_map::MapObject { file, id, pos: [100.0 + id as f64 * 10.0, 100.0, 0.0], ..Default::default() });
    }
    crate::doc::save(&mut doc).unwrap();

    let zip = base.join("ExportTest.zip");
    let (plan, bytes) = export(&doc, &roots, &zip).unwrap();
    assert!(bytes > 0);
    let names: Vec<&str> = plan.files.keys().map(|s| s.as_str()).collect();
    let has = |part: &str| names.iter().any(|n| n.to_lowercase().contains(&part.to_lowercase()));
    assert!(has("maps/ExportTest/global.cfg") && has("maps/ExportTest/tile_0_0.map"), "{names:?}");
    assert!(has("Imported/kiosk/kiosk.sco") && has("kiosk/model/kiosk_") && has("kiosk/texture/kiosk.png") && has("kiosk_nn.png"), "{names:?}");
    // (and the source openMapper edits it from, so the map's recipient can too)
    assert!(has("kiosk/kiosk.openmapper.json"), "{names:?}");
    assert!(!has("Generic"), "stock files are not packed: {names:?}");
    assert!(plan.stock.keys().any(|k| k.eq_ignore_ascii_case("Sceneryobjects\\Generic")));
    assert!(plan.missing.is_empty(), "{:?}", plan.missing);
    let text = readme(&doc, &plan);
    assert!(text.contains("Export test") && text.contains("A map to share.") && text.contains("Sceneryobjects\\Generic"));

    // into a fresh content folder: the map and its model are there, nothing is missing
    crate::import::import(&zip, &c2).unwrap();
    let roots2 = Roots { omsi: omsi.clone(), content: c2.clone() };
    let cfg = c2.join("maps").join("ExportTest").join("global.cfg");
    let back = Document::load(&roots2, &cfg, |_| {}).unwrap();
    assert_eq!(back.tiles.len(), 1);
    let dirs = [c2.clone(), omsi.clone()];
    for o in back.tiles.values().flat_map(|t| t.tile.objects.iter()) {
        let p = find_in(&c2, &o.file).or_else(|| find_in(&omsi, &o.file)).unwrap_or_else(|| panic!("{} missing", o.file));
        assert!(crate::check::missing_textures(&p, false, &dirs).is_empty(), "{}", o.file);
    }
    let _ = std::fs::remove_dir_all(&base);
}


#[test]
fn restricted_export_round_trips_as_a_dependency_without_copying_publisher_bytes() {
    let _guard = EXPORT_ZIP_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let _map = crate::doc::one_map();
    let base = std::env::temp_dir().join(format!("openmapper-reference-export-{}", std::process::id()));
    let (first, second) = (base.join("first"), base.join("second"));
    for root in [&first, &second] { std::fs::create_dir_all(root.join("Packs")).unwrap(); }
    let signer = omsi_pack::SigningKey::generate(&ring::rand::SystemRandom::new()).unwrap();
    let mut trust = omsi_pack::TrustedPublishers::default();
    trust.insert("export-fixture", signer.key_id(), signer.public_key());
    let bytes = b"[friendlyname]\nOwned export fixture\n";
    let relative = "Sceneryobjects/ReferenceFixture/renamed.sco";
    for root in [&first, &second] {
        trust.save_file(&root.join("Publishers.json")).unwrap();
        omsi_pack::create_pack(&root.join("Packs/objects.oopp"), omsi_pack::PackMetadata {
            publisher_id: "export-fixture".into(), author: "Fixture".into(), pack_id: "objects".into(),
            version: "1.0.0".into(), license: "LicenseRef-Owned-Fixture".into(), redistribution: false,
            key_id: None, dependencies: vec![],
        }, &[omsi_pack::AssetInput { id: "object".into(), path: "Sceneryobjects/ReferenceFixture/original.sco".into(),
            source_sha256: omsi_pack::sha256_hex(b"original owned source"), bytes: bytes.to_vec() }],
            &signer, None, &std::sync::atomic::AtomicBool::new(false)).unwrap();
    }
    let owned = first.join(relative);
    std::fs::create_dir_all(owned.parent().unwrap()).unwrap();
    std::fs::write(&owned, bytes).unwrap();
    let roots = Roots { omsi: base.join("stock"), content: first.clone() };
    let mut doc = Document::new_map(&roots, "References", "References", 1);
    // This owned fixture has no installation template to copy.
    doc.source = None;
    doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects.push(omsi_map::MapObject {
        file: relative.replace('/', "\\"), id: 1, pos: [100.0, 100.0, 0.0], ..Default::default()
    });
    crate::doc::save(&mut doc).unwrap();
    let zip = base.join("references.zip");
    let (planned, _) = export(&doc, &roots, &zip).unwrap();
    assert!(!planned.files.contains_key(relative), "restricted source was copied by the actual exporter");
    assert!(readme(&doc, &planned).contains("LicenseRef-Owned-Fixture"));
    let (declared, _) = export_with_rights(&doc, &roots, &base.join("rights.zip"), true).unwrap();
    assert!(declared.files.contains_key(relative));
    assert!(declared.references.assets.is_empty());
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip).unwrap()).unwrap();
    assert!(archive.by_name(relative).is_err());
    assert!(archive.by_name("maps/References/dependencies.json").is_ok());
    drop(archive);
    crate::import::import(&zip, &second).unwrap();
    assert!(!second.join(relative).exists());
    omsi_cfg::add_content_root(second.clone());
    let roots2 = Roots { omsi: roots.omsi.clone(), content: second.clone() };
    let back = Document::load(&roots2, &second.join("maps/References/global.cfg"), |_| {}).unwrap();
    let alias = omsi_cfg::resolve_path(&back.dir, relative);
    assert!(omsi_scenery::SceneryObject::load(&alias).is_ok());
    assert_eq!(omsi_cfg::vfs::read(&alias).unwrap(), bytes);
    // Re-export an owned overlay without losing the references it already carries.
    let second_zip = base.join("again.zip");
    export(&back, &roots2, &second_zip).unwrap();
    let mut archive = zip::ZipArchive::new(std::fs::File::open(second_zip).unwrap()).unwrap();
    assert!(archive.by_name("maps/References/dependencies.json").is_ok());
    drop(archive);
    let physical = second.join(relative);
    std::fs::create_dir_all(physical.parent().unwrap()).unwrap();
    std::fs::write(&physical, bytes).unwrap();
    let (overridden, _) = export_with_rights(&back, &roots2, &base.join("overlay-rights.zip"), true).unwrap();
    assert!(overridden.references.assets.is_empty());
    assert!(overridden.references.dependencies.is_empty());
    let install = base.join("install");
    let stale = install.join("maps/References/dependencies.json");
    std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
    std::fs::write(&stale, b"old dependency metadata").unwrap();
    install_into(&overridden, &install).unwrap();
    assert!(!stale.exists());
    drop(back);
    omsi_cfg::remove_content_root(&second);
    std::fs::remove_dir_all(base).unwrap();
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn mapper_publish_export_import_opens_plain_and_encrypted_world_assets() {
    let _guard = EXPORT_ZIP_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let _map = crate::doc::one_map();
    for protected in [false, true] {
        publisher_world_round_trip(protected);
    }
}

#[cfg(any(target_os = "macos", windows))]
fn publisher_world_round_trip(protected: bool) {
    struct Fixture {
        base: PathBuf,
        content_root: Option<PathBuf>,
        unlocked_pack: Option<PathBuf>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(pack) = &self.unlocked_pack {
                let trusted_file = self.base.join("recipient-content/Publishers.json");
                if let Ok(trusted) = omsi_pack::TrustedPublishers::load_file(&trusted_file) {
                    let _ = omsi_pack::forget_unlock(pack, &trusted, &omsi_pack::SystemKeyStore);
                }
            }
            if let Some(root) = &self.content_root {
                omsi_cfg::remove_content_root(root);
            }
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "openmapper-world-publish-{}-{stamp}-{}",
        std::process::id(),
        protected as u8
    ));
    let first = base.join("publisher-content");
    let second = base.join("recipient-content");
    let source = base.join("source");
    let mut fixture = Fixture {
        base: base.clone(),
        content_root: None,
        unlocked_pack: None,
    };
    std::fs::create_dir_all(first.join("Packs")).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    std::fs::create_dir_all(&source).unwrap();

    // Import an owned OBJ plus material texture through openMapper's actual model importer.
    std::fs::write(
        source.join("kiosk.mtl"),
        "newmtl kiosk\nKd 1 1 1\nmap_Kd kiosk.png\n",
    )
    .unwrap();
    std::fs::write(
        source.join("kiosk.obj"),
        "mtllib kiosk.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nusemtl kiosk\nf 1/1 2/2 3/3\n",
    )
    .unwrap();
    let texel = [23, 97, 181, 255];
    image::RgbaImage::from_pixel(16, 16, image::Rgba(texel))
        .save(source.join("kiosk.png"))
        .unwrap();
    let imported = crate::import::import(&source.join("kiosk.obj"), &first).unwrap();
    let imported_object = find_in(&first, &imported.objects[0]).unwrap();
    let imported_dir = imported_object.parent().unwrap();
    let imported_mesh = std::fs::read_dir(imported_dir.join("model"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|extension| extension == "x"))
        .unwrap();
    let decoded_mesh = omsi_o3d::load_mesh(&imported_mesh).unwrap();
    let texture_name = decoded_mesh
        .materials
        .iter()
        .map(|material| material.texture.trim().replace('\\', "/"))
        .find(|name| !name.is_empty())
        .and_then(|name| name.rsplit('/').next().map(str::to_owned))
        .unwrap();
    let imported_texture = imported_dir.join("texture").join(&texture_name);
    assert!(imported_texture.is_file(), "{imported_texture:?}");

    let object_rel = "Sceneryobjects/Fixture/renamed.sco";
    let mesh_rel = format!(
        "Sceneryobjects/Fixture/model/{}",
        imported_mesh.file_name().unwrap().to_string_lossy()
    );
    let texture_rel = format!("Sceneryobjects/Fixture/texture/{texture_name}");
    let output_object = first.join(object_rel);
    let output_mesh = first.join(&mesh_rel);
    let output_texture = first.join(&texture_rel);
    for path in [output_object.as_path(), output_mesh.as_path(), output_texture.as_path()] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    }
    std::fs::copy(&imported_object, &output_object).unwrap();
    std::fs::copy(&imported_mesh, &output_mesh).unwrap();
    std::fs::copy(&imported_texture, &output_texture).unwrap();

    let signer = omsi_pack::SigningKey::generate(&ring::rand::SystemRandom::new()).unwrap();
    let signing_file = base.join("publisher.pk8");
    signer.save_private_key(&signing_file).unwrap();
    let pack_id = if protected { "encrypted-model" } else { "plain-model" };
    let pack = first.join("Packs").join(format!("{pack_id}.oopp"));
    let content_key = base.join("content-key.bin");
    let source_obj = source.join("kiosk.obj");
    let source_png = source.join("kiosk.png");
    let spec = base.join("publish.json");
    let source_relative = |path: &Path| {
        path.strip_prefix(&base)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/")
    };
    std::fs::write(
        &spec,
        serde_json::to_vec(&serde_json::json!({
            "protected": protected,
            "metadata": {
                "publisher_id": "openmapper-runtime-fixture",
                "author": "Fixture",
                "pack_id": pack_id,
                "version": "1.0.0",
                "license": "LicenseRef-Owned-Fixture",
                "redistribution": false,
                "key_id": null,
                "dependencies": []
            },
            "assets": [
                {
                    "id": "object",
                    "path": "Sceneryobjects/Fixture/original.sco",
                    "file": source_relative(&imported_object),
                    "source": source_relative(&source_obj)
                },
                {
                    "id": "mesh",
                    "path": mesh_rel.clone(),
                    "file": source_relative(&imported_mesh),
                    "source": source_relative(&source_obj)
                },
                {
                    "id": "texture",
                    "path": texture_rel.clone(),
                    "file": source_relative(&imported_texture),
                    "source": source_relative(&source_png)
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let manifest = crate::publisher::publish(
        &spec,
        &pack,
        &signing_file,
        protected.then_some(content_key.as_path()),
    )
    .unwrap();

    let mut trusted = omsi_pack::TrustedPublishers::default();
    trusted.insert(
        "openmapper-runtime-fixture",
        signer.key_id(),
        signer.public_key(),
    );
    trusted.save_file(&first.join("Publishers.json")).unwrap();

    let roots = Roots {
        omsi: base.join("stock"),
        content: first.clone(),
    };
    let mut doc = Document::new_map(&roots, "RuntimeFixture", "Runtime fixture", 1);
    doc.source = None;
    doc.tiles.get_mut(&(0, 0)).unwrap().tile.objects.push(omsi_map::MapObject {
        file: object_rel.replace('/', "\\"),
        id: 1,
        pos: [100.0, 100.0, 0.0],
        ..Default::default()
    });
    crate::doc::save(&mut doc).unwrap();

    let zip_path = base.join("RuntimeFixture.zip");
    let (plan, _) = export(&doc, &roots, &zip_path).unwrap();
    assert!(plan.errors.is_empty(), "{:?}", plan.errors);
    assert!(plan.references.assets.iter().any(|asset| asset.asset_id == "object"));
    assert!(plan.references.assets.iter().any(|asset| asset.asset_id == "mesh"));
    assert!(plan.references.assets.iter().any(|asset| asset.asset_id == "texture"));
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
    assert!(archive.by_name(object_rel).is_err());
    assert!(archive.by_name(&mesh_rel).is_err());
    assert!(archive.by_name(&texture_rel).is_err());
    let dependencies_name = "maps/RuntimeFixture/dependencies.json";
    let mut dependencies_file = archive.by_name(dependencies_name).unwrap();
    let mut dependencies_bytes = Vec::new();
    std::io::Read::read_to_end(&mut dependencies_file, &mut dependencies_bytes).unwrap();
    let dependencies: omsi_cfg::publisher::MapReferences =
        serde_json::from_slice(&dependencies_bytes).unwrap();
    assert!(dependencies.assets.iter().any(|asset| asset.asset_id == "object"));
    assert!(dependencies.assets.iter().any(|asset| asset.asset_id == "mesh"));
    assert!(dependencies.assets.iter().any(|asset| asset.asset_id == "texture"));
    drop(dependencies_file);
    drop(archive);

    // The recipient installs the signed pack independently, then imports the actual map ZIP.
    std::fs::create_dir_all(second.join("Packs")).unwrap();
    std::fs::rename(&pack, second.join("Packs").join(pack.file_name().unwrap())).unwrap();
    std::fs::copy(
        first.join("Publishers.json"),
        second.join("Publishers.json"),
    )
    .unwrap();
    crate::import::import(&zip_path, &second).unwrap();
    assert!(!second.join(object_rel).exists());
    assert!(!second.join(&mesh_rel).exists());
    assert!(!second.join(&texture_rel).exists());
    let recipient_pack = second.join("Packs").join(pack.file_name().unwrap());
    if protected {
        fixture.unlocked_pack = Some(recipient_pack.clone());
        let key: [u8; 32] = std::fs::read(&content_key).unwrap().try_into().unwrap();
        let expires = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let entitlement = omsi_pack::issue_entitlement(&manifest, key, expires, &signer).unwrap();
        omsi_pack::unlock(
            &recipient_pack,
            &entitlement,
            &trusted,
            &omsi_pack::SystemKeyStore,
        )
        .unwrap();
    }
    omsi_cfg::add_content_root(second.clone());
    fixture.content_root = Some(second.clone());

    let map_dir = second.join("maps/RuntimeFixture");
    let world = omsi_scene::World::open(
        &second,
        &map_dir.join("global.cfg"),
        20261001,
    )
    .unwrap();
    let object_virtual = omsi_cfg::resolve_path(&map_dir, object_rel);
    assert!(omsi_cfg::vfs::is_file(&object_virtual));
    assert!(!object_virtual.exists(), "the object must stay in the signed virtual mount");
    let object = world
        .object_type(object_rel)
        .expect("the imported map resolves its signed publisher object alias");
    assert!(
        object.meshes.iter().any(|(mesh, _, _)| mesh.indices.len() >= 3),
        "World::object_type must decode the imported mesh"
    );
    let mesh_virtual = omsi_cfg::resolve_path(&map_dir, &mesh_rel);
    let texture_virtual = omsi_cfg::resolve_path(&map_dir, &texture_rel);
    assert!(omsi_cfg::vfs::is_file(&mesh_virtual));
    assert!(omsi_cfg::vfs::is_file(&texture_virtual));
    assert!(!mesh_virtual.exists(), "the mesh must stay in the signed virtual mount");
    assert!(!texture_virtual.exists(), "the texture must stay in the signed virtual mount");
    let decoded = omsi_texture::decode_file(&texture_virtual).unwrap();
    assert_eq!((decoded.width, decoded.height), (16, 16));
    assert_eq!(&decoded.rgba[..4], &texel[..]);

    if protected {
        assert!(omsi_cfg::vfs::is_sealed(&texture_virtual));
        let cache_dir = base.join("protected-texture-cache");
        let cache = omsi_texture::disk_cache::DiskCache::new(cache_dir.clone(), 1024 * 1024);
        let bytes = omsi_cfg::vfs::read(&texture_virtual).unwrap();
        let options = omsi_texture::gpu::GpuOptions {
            bc: true,
            compress: true,
        };
        let _ = omsi_texture::gpu::load_gpu_cached(
            &bytes,
            &texture_virtual,
            options,
            Some(&cache),
        )
        .unwrap();
        assert!(!cache_dir.exists(), "protected texture must bypass the disk cache");
    } else {
        assert!(!omsi_cfg::vfs::is_sealed(&texture_virtual));
    }
}


#[test]
fn a_failed_export_leaves_no_partial_archive() {
    let root = std::env::temp_dir().join(format!("openmapper-export-cleanup-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let out = root.join("output.zip");
    std::fs::write(&out, b"existing user archive").unwrap();
    let mut plan = Plan::default();
    plan.files.insert("Sceneryobjects/missing.sco".into(), root.join("missing.sco"));
    assert!(write_zip(&plan, "owned fixture", &out).is_err());
    assert_eq!(std::fs::read(&out).unwrap(), b"existing user archive");
    assert!(!out.with_extension("zip.openmapper-new").exists());
    std::fs::remove_dir_all(root).unwrap();
}

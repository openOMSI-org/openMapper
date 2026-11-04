# Optional publisher content

Ordinary maps need no publisher files or account. Signed publisher packs add provenance,
licence metadata and exact dependency references. Optional encryption adds protected playback
for openOMSI. These are separate policies: an ordinary rights declaration never unlocks a pack.

## Publisher authoring

Use owned plaintext authoring inputs. Generate a disposable test signer with the companion
`omsi-pack keygen signing.pk8 Publishers.json example` command. Keep the private key and all
content keys outside distributed content. Players obtain and explicitly trust the public
publisher file through an independent channel; a signature alone does not prove ownership.

`openmapper --publish-pack spec.json scenery.oopp signing.pk8 [new-content-key.bin]`
creates a new pack directory without replacing an existing output. The optional last argument
is required when `protected` is true. Each protected publication generates a fresh random key
and key ID. The source and playable paths are relative to the specification folder:

```json
{"protected":true,"metadata":{"publisher_id":"example","author":"Example publisher","pack_id":"scenery","version":"1.0.0","license":"LicenseRef-Example","redistribution":false,"key_id":null,"dependencies":[]},"assets":[{"id":"sign","path":"Sceneryobjects/Example/model/sign.x","file":"converted/sign.x","source":"original/sign.obj"}]}
```

Import the original with openMapper first to obtain converted playable files. The publisher
is responsible for pairing each original with its actual converted output; signing a manifest
is a publisher claim, not independent verification of authorship. Byte hashes are computed
before import conversion and retained in the local provenance index. Original and converted
hashes are separate fields in the signed pack. Pack creation consumes explicit authoring
files and cannot use protected playback assets as editing sources.

Install `.oopp` directories under `content/Packs` and explicitly configure `content/Publishers.json`.
For protected packs the publisher uses the companion offline issuer:

```text
omsi-pack issue scenery.oopp signing.pk8 new-content-key.bin <expiry-unix> entitlement.json
omsi-pack unlock scenery.oopp entitlement.json Publishers.json
omsi-pack inspect scenery.oopp Publishers.json
```

Deliver the entitlement confidentially. It contains a bearer secret and authorizes the exact
signed manifest/version until expiry. Unlock stores the signed envelope in macOS Keychain or
Windows user-scoped DPAPI. No purchase service, universal key or client ownership flag exists.
Back up publisher keys privately; users obtain replacement entitlements if native storage is
lost. Linux protected storage is currently unsupported; signed unencrypted packs still work.

## Per-sale packs and licences

A publisher selling a map can build one protected pack and one licence for each sale:

```text
openmapper --publish-licensed-packs spec.json sales signing.pk8 licence-ids.txt <expiry-unix>
```

- **Licence IDs.** `licence-ids.txt` lists one opaque licence ID per line, for example 32
  random hexadecimal characters from the publisher's store. IDs use 1 to 128 ASCII letters,
  digits, dots, underscores or hyphens, the same format as
  [placement watermark](PLACEMENT_WATERMARKS.md) buyer IDs, so one ID can name both. They
  hold no personal data; only the publisher's own records link an ID to a buyer.
- **What is built.** For each ID, the new `sales` folder gets:
  - `<id>.oopp`, a pack with a fresh content key whose signed manifest names the ID;
  - `<id>.licence.json`, a licence for that pack that repeats the ID.
- **What goes where.** Send each buyer only their `.oopp` folder and `.licence.json` file.
  Keep `<id>.key` and `licences.json` private. The key lets the publisher issue a
  replacement licence later with `omsi-pack issue ... --licence-id <id>`. `licences.json`
  records each licence ID with its pack, version, manifest digest, key ID and expiry.
- **What it does.** A licence unlocks only the pack built for the same ID, so a leaked pack
  or licence names the sale it came from. Original-file hashes are the same in every
  buyer's pack, so provenance recognition still works.
- **Failures.** The specification must be protected. A repeated, malformed or case-only
  duplicate ID, a past expiry or any build failure leaves no output folder.

Licence IDs trace a pack or licence that was passed on as it is. A copy taken from a
decrypted map in memory carries no licence ID; placement watermarks address that case.

## Sharing a map

Normal export consults trusted signed manifests and the retained `PackIndex`. Exact matching
restricted outputs become `dependencies.json` references beside `global.cfg`, with publisher,
pack/version and stable asset ID. Renaming a file or removing a nearby manifest does not hide
it while the trusted index remains. Modified bytes and deletion of every trusted record cannot
be recognized by exact hashes. Export hashes used files and consults the index, rather than
scanning an entire asset library. The local conversion index is a provenance hint, not a
publisher signature or ownership proof.

The archive readme lists attribution, licences and requirements. Install the referenced packs
separately; the runtime reports missing, wrong-version, untrusted or changed packs. Owned maps
can retain references as overlays. Protected maps and assets support playback, and official
editing, import, save, autosave, conversion, texture caching and export paths refuse plaintext
copies. A modified player can bypass those policies.

The export dialog offers dependency references or an explicit redistribution-rights declaration
for ordinary restricted files. Protected assets remain references and never gain an editing
or plaintext-export exception. Cancellation creates no archive.

For ordinary redistribution metadata only, `openmapper --export path/to/global.cfg out.zip
--i-hold-the-rights` explicitly declares the exporter's redistribution rights. This keeps
warnings and never supplies a key or validates an invalid signature. Converted restricted
content without an exact playable published output needs that output or a rights declaration.

## Limits and optional fingerprints

See [the shared format](https://github.com/Italian-seasoning/openOMSI/blob/feat/publisher-content-protection/docs/PUBLISHER_PACKS.md)
for canonical signature bytes, published hash lists, path and size limits, authenticated chunk
layout, and dependency resolution. No policy website or community integration is deployed.

The offline player exposes usable assets to its own process, GPU and operating system paging.
An administrator or modified client can capture them. Offline expiry depends on the machine's
clock; immediate revocation of already cached keys is unavailable. Encryption reduces casual
extraction through official tools and cannot guarantee unextractable playable content.

[The optional fingerprint utility](FINGERPRINTS.md) embeds opaque identifiers into owned OBJ
geometry or binary PPM texture samples. Its tests document supported transformations and
false-positive limits. It is separate from signatures and encryption, has no telemetry, and
is not a durable watermark against arbitrary modification.

[The optional placement watermark](PLACEMENT_WATERMARKS.md) gives each buyer's copy of a map
its own sub-millimetre object placements, so the creator can trace a leaked copy to the buyer
it was made for. It needs the creator's secret key and the unmarked original to inspect a copy.

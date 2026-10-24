# Interface languages

The editor supports English, Português (Brasil), Português (Portugal) and Русский.
The selected language is saved in preferences. When no language is set, the
operating-system locale (including the Windows region) selects PT-BR/PT-PT or
Russian when available; otherwise English is used.

Existing `tr("English", "Русский")` calls keep Russian inline, so no separate
Russian or English catalogue is needed. The two embedded UTF-8 Portuguese
catalogues are `locales/pt-BR.json` and `locales/pt-PT.json`. No network or
machine translation service is used when the editor runs.

## Explicitly partial catalogues

The Portuguese catalogues prioritize the main File/Edit/View/Tools/Map/Help
menus (including all visible Map actions), the command palette, frequent tools,
errors and guidance. These remain deliberately partial catalogues. Missing labels intentionally
fall back to English. This avoids presenting the original large set of
unreviewed automatic translations as accepted UI copy; thorough native-speaker
review and in-app layout acceptance remain outstanding.

Run `python3 tools/check-locales.py` whenever changing a label. CI now runs
the checker and rejects stale catalogue keys, blank translations, generated
token markers and missing protected literals. New untranslated source labels
use English fallback until deliberately added. Both Portuguese files keep
the same keys; core UI labels must have translations.

## Windows x64 preview for PR #12

Open this PR's **CI** run, download the **openMapper-PR12-Windows-x64**
artifact from the Artifacts section, and extract both the Actions ZIP and the
included application ZIP. Run `openmapper.exe` (Windows x64).
This is a test build and requires OMSI 2 to edit its maps.
Switch languages in **View > Language**. Capture and share screenshots of
menus, toolbar, palette, and guidance/errors in both Portuguese variants.
Note incorrect words and clipped text; unreviewed entries will appear in
English by design. Artifacts are retained for 14 days.

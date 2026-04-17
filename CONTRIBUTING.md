# Contributing to mush

Thanks for your interest in improving `mush`.

## Before opening a PR

- Open an issue first for larger changes, new features, or UX shifts.
- Keep changes focused and easy to review.
- Prefer fixes at the DSP/state-transition level over UI-only masking.
- Avoid committing generated files except the checked-in demos.

## Development notes

- The source of truth is the embedded Python inside `mu.sh`.
- The launcher writes runtime files to `$HOME/.mush.py` and `$HOME/.mush-venv`.
- Audio output may not be available in CI or headless environments.
- Camera visuals depend on `ffmpeg` and may not work on every machine.

## Validation

Run these before submitting:

```bash
bash -n mu.sh
python3 - <<'PY'
from pathlib import Path
text = Path('mu.sh').read_text()
start = text.index("<< 'PYEOF'\n") + len("<< 'PYEOF'\n")
end = text.index("\nPYEOF", start)
compile(text[start:end], 'embedded_mush.py', 'exec')
print('ok')
PY
```

If your change affects runtime behavior, include manual test notes in the PR.

## Style

- Preserve the current single-file structure unless there is a strong reason not to.
- Keep terminal UX concise and discoverable.
- Do not hardcode machine-local audio or MIDI device names into shared defaults.
- Document user-facing control changes in `README.md`.

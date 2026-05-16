# Model Cache Operations

Quaid stores downloaded extraction and embedding models under the shared model
cache root:

- `QUAID_MODEL_CACHE_DIR` when set
- otherwise `~/.quaid/models`

Use `quaid model status` to inspect cache health without starting a download.
By default, status uses fast manifest and file metadata checks. Add `--verify`
when you want full hash verification:

```bash
quaid model status
quaid model status phi-3.5-mini --verify
quaid model status small --verify
```

Status reports each known cache entry as one of:

- `complete`: required files and metadata are present
- `missing`: an alias-specific cache path is absent
- `incomplete`: required files or manifest metadata are missing
- `corrupted`: manifest or hash validation failed
- `stale-temp`: a temporary download path is old enough to remove
- `active-temp`: a temporary download still has a fresh heartbeat or mtime

Use `quaid model clean --list` before deleting anything:

```bash
quaid model clean --list
```

Use `--all` to remove stale temporary paths and invalid partial caches. Complete
verified caches are preserved by broad cleanup.

```bash
quaid model clean --all
quaid model clean --all --force
```

Use an alias-specific clean when you intentionally want to remove a complete
cache and re-pull it:

```bash
quaid model clean phi-3.5-mini
quaid model clean phi-3.5-mini --force
```

Temporary extraction downloads write a `.downloading` heartbeat while files are
streaming. Quaid treats temporary paths as active while the heartbeat or path
mtime is fresh. The stale threshold defaults to 6 hours and can be overridden
with:

```bash
QUAID_STALE_MODEL_CACHE_TTL_SECS=3600 quaid model clean --list
```

If a download fails, Quaid removes its temporary files or directories
automatically. If cleanup itself fails because of permissions or a locked file,
the error message includes the path and the matching `quaid model clean`
command to run after the process exits.

# Integraciones live de OpenCode

Las seis integraciones live de Rust están marcadas `#[ignore]` y no se ejecutan en la suite normal. Para opt-in, configura un OpenCode local con un proveedor disponible y directorios temporales dedicados; no uses un checkout ni datos de usuario, porque las pruebas crean sesiones, envían prompts y el ciclo de review modifica archivos en el primer directorio.

Desde la raíz del repositorio:

```sh
export MARVIS_AGENT_BRIDGE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/marvis-agent-bridge-a.XXXXXX")"
export MARVIS_AGENT_BRIDGE_OTHER_DIR="$(mktemp -d "${TMPDIR:-/tmp}/marvis-agent-bridge-b.XXXXXX")"
cargo test --locked --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture --test-threads=1
```

Ese comando selecciona las seis pruebas ignoradas actuales: cinco de `services::agent` y `services::review_round::tests::a_queued_round_goes_out_when_flushed`. `MARVIS_AGENT_BRIDGE_DIR` es obligatorio en todas; `MARVIS_AGENT_BRIDGE_OTHER_DIR`, en las dos pruebas de dos checkouts. Deben ser directorios distintos, temporales y exclusivos para esta ejecución. Si falta una variable requerida, el test falla con el nombre de la variable en vez de pasar sin ejecutarse. No hace falta `MARVIS_AGENT_BRIDGE=1`: `--ignored` es el opt-in explícito.

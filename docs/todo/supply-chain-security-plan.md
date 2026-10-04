# Plan: seguridad de dependencias y toolchain

**Estado:** plan aprobado; pendiente de implementación. Los objetivos son un inventario del 2 de octubre de 2026 y ya no coinciden con el árbol actual: el toolchain está fijado en `rust-toolchain.toml` (1.97.1, no 1.99.0), la auditoría de Rust usa `scripts/audit-rust.mjs` en lugar de `cargo audit`, y las versiones instaladas son Vitest 4.1.11 y Vite 7. Verificá cada punto contra el árbol antes de aplicarlo.  
**Inventario consultado:** 2 de octubre de 2026.  
**Alcance:** pnpm, Cargo/Rust, configuración de OpenCode y GitHub Actions; actualizar dependencias directas a la versión estable más reciente que soporte el toolchain y no viole las políticas del repo.

## Decisiones

- Activar gates `pnpm audit --audit-level moderate` y `cargo audit` después de resolver los advisories actuales.
- Actualizar dependencias directas a lo último compatible, incluyendo majors cuando sus peers y gates lo permitan.
- Mantener TypeScript en **5.9.3**: `typescript-eslint@8.71.0` (latest estable y canary consultado) declara `typescript <6.1.0`; no hay TypeScript 6 estable. TypeScript 7 rompería el lint configurado en `eslint.config.js`.
- Actualizar `rusqlite` a **0.40.2** en este mismo lote. La revisión de usos no encontró las APIs afectadas por sus cambios incompatibles (módulo VTab, `ValueRef`, statement cache opcional, hooks ni `u64`/`usize` SQL).
- Fijar Rust en `1.99.0`; fijar acciones GitHub por SHA completo.
- Mantener fuera de alcance: AGENTS.md, Dependabot y attestations/provenance.

## Hallazgos que motivan el trabajo

- `opencode.json` ejecuta `npx -y @hypothesi/tauri-mcp-server` sin versión ni lockfile: cada arranque puede traer una versión distinta.
- Auditoría pnpm inicial: 3 advisories moderados, todos dev/optional; 2 corresponden a Vitest (`GHSA-82fw-gwwq-j7x9`) y 1 a `decode-uri-component` (`GHSA-vcc3-ghjq-m6fr`), transitivo de `vite > stylus > css > source-map-resolve`.
- pnpm 12 ya aplica builds deny-by-default, `blockExoticSubdeps`, integridad de store y edad mínima por defecto. Sin embargo, el default de `minimumReleaseAge` no es strict; declararlo explícitamente activa el comportamiento estricto.
- El lockfile pnpm incluye integridad SHA-512 y no contiene dependencias git/file ni URLs tarball. `Cargo.lock` está versionado y no incluye fuentes git.
- `pnpm outdated` detectó upgrades directas de Vite/Vitest/ESLint/SVGO y upgrades menores de Tauri, Shiki, Lucide y Lezer. `vite@8` usa Rolldown; los plugins Vue y Tailwind existentes aceptan Vite 8.

## Secuencia de implementación

### 0. Baseline

Antes de modificar dependencias, ejecutar y guardar el resultado de:

```sh
pnpm install --frozen-lockfile
pnpm test
pnpm lint
pnpm typecheck
pnpm build:app
```

Si falla el baseline, registrar los fallos existentes y no atribuirlos a los upgrades.

### 1. Fortalecer resolución y fijar herramientas

- En `pnpm-workspace.yaml`, declarar `minimumReleaseAge: 1440` y `trustPolicy: no-downgrade`; conservar `trustLockfile: false` (default deliberado para revalidar el lockfile en instalaciones públicas).
- Eliminar los 13 `minimumReleaseAgeExclude` puntuales de Tauri luego de actualizar las versiones referenciadas: las versiones objetivo comprobadas ya tienen más de 24 horas. Si pnpm requiere una excepción, agregar solo el paquete exacto y justificarla en revisión; no ampliar exclusiones con comodines.
- Añadir temporalmente el override `decode-uri-component: 0.5.0` para `GHSA-vcc3-ghjq-m6fr`, cuyo rango requerido no cabe en `source-map-resolve@0.6.0` (`^0.2.0`). Tras Vite 8, ejecutar `pnpm why decode-uri-component`; eliminar el override si el árbol ya no contiene el paquete vulnerable.
- Añadir `@hypothesi/tauri-mcp-server@0.13.0` como devDependency y cambiar `opencode.json` a `pnpm exec mcp-server-tauri`, de modo que quede en el lockfile y bajo las políticas de instalación.
- Crear `src-tauri/rust-toolchain.toml` con `channel = "1.99.0"`, `components = ["rustfmt", "clippy"]`.
- Añadir `--locked` a los scripts Cargo de `package.json` (`lint:rust`, `test:rust`, `test:security`, `test`); no aplicarlo a `cargo fmt`, que no resuelve dependencias.

### 2. Resolver advisories npm y actualizar frontend

- `vitest`: 3.2.7 → **5.0.3**; resuelve el advisory de path traversal. En Vitest 5 `clearMocks` default es `true`, así que se ejecuta `vi.clearAllMocks()` antes de cada test. Mantener los resets explícitos existentes por ahora y correr toda la suite; corregir solo tests que dependan de llamadas acumuladas entre tests.
- `vite`: 7.3.6 → **8.3.2** (Rolldown). Validar dev server, bundle de producción y transformaciones de Vitest.
- ESLint: `eslint` 9.39.5 → **10.11.0**, `@eslint/js` → **10.0.1**, `globals` → **17.13.0**, `eslint-plugin-vue` → **10.11.1**, `typescript-eslint` → **8.71.0**. Revisar y corregir nuevas reglas reportadas por el lint.
- Mantener `typescript` en **5.9.3** hasta que `typescript-eslint` soporte TS 7; actualizar `vue-tsc` a **3.3.12** cuando supere la ventana mínima de 24 horas.
- Actualizar SVG optimizer `svgo` 3.3.5 → **4.1.0**. Solo lo usa el script manual de generación de iconos; ejecutar ese script con un checkout Catppuccin disponible y revisar la salida generada si el entorno lo permite.
- Actualizar los menores compatibles observados: `@lezer/highlight` 1.2.4 → 1.2.5; `@tauri-apps/api` y `@tauri-apps/cli` 2.12.0 → 2.12.1; plugin clipboard 2.4.0 → 2.4.1; plugin dialog 2.7.3 → 2.8.1; `@lucide/vue` 1.48.0 → última versión madura (1.49.0 al inventariar; 1.50.0 retenida por edad); `shiki` 4.4.3 → 4.5.0.
- Mantener `allowBuilds` con su allowlist actual, y no ejecutar `pnpm approve-builds` para aceptar scripts sin revisión.

### 3. Actualizar Rust

- En `src-tauri/Cargo.toml`, subir `rusqlite` 0.37 → **0.40.2** (mantener `bundled`) y `base64` 0.22 → **0.23.1**. El cambio de base64 añade SIMD detrás de `simd-unsafe`, habilitado por default; decidir si se conserva o se usa `default-features = false` para no introducir código unsafe de la dependencia.
- Ejecutar `cargo update` para los 31 upgrades que permanecen dentro de los rangos semver actuales y revisar el diff completo de `Cargo.lock`.
- Añadir Cargo Audit a CI con `cargo install cargo-audit --locked` y `cargo audit --locked` (la sintaxis final debe ajustarse a la CLI instalada; `cargo audit` consume el `Cargo.lock` versionado).

### 4. Gates de CI y permisos mínimos

- SHA-pinear cada `uses:` de `.github/workflows/ci.yml` y `release.yml`, manteniendo un comentario con el tag legible. SHA consultados:

  | Acción                         | SHA                                        |
  | ------------------------------ | ------------------------------------------ |
  | `actions/checkout@v7`          | `3d3c42e5aac5ba805825da76410c181273ba90b1` |
  | `actions/setup-node@v7`        | `820762786026740c76f36085b0efc47a31fe5020` |
  | `pnpm/action-setup@v6`         | `0977fd99725f1db4007ccb2928dbb4e90d06cc86` |
  | `swatinem/rust-cache@v2`       | `6323deb102c322ba6fcbdcafc7e3dddab59af2b6` |
  | `actions/upload-artifact@v4`   | `ea165f8d65b6e75b540449e92b4886f43607fa02` |
  | `actions/download-artifact@v4` | `d3f86a106a0bac45b974a628896c90dbdf5c8093` |

- Quitar `dtolnay/rust-toolchain@stable` (es una branch mutable). Usar el toolchain fijado por `rust-toolchain.toml` y añadir `rustup target add ${{ matrix.target }}` en los jobs que compilan targets cruzados.
- En ambos jobs de `ci.yml`, declarar `permissions: contents: read`; en checkout configurar `persist-credentials: false` donde no se requiera push.
- Añadir `pnpm audit --audit-level moderate` y `cargo audit` tras resolver los advisories/upgrades y confirmar un baseline limpio.

## Verificación y aceptación

- `pnpm install --frozen-lockfile` termina sin cambios del lockfile y sin aceptar scripts de build nuevos sin revisión explícita.
- `pnpm audit --audit-level moderate` y `cargo audit` no reportan advisories moderados o superiores sin excepción documentada.
- Pasan `pnpm fmt:check`, `pnpm lint`, `pnpm typecheck`, `pnpm test`, `pnpm test:security`, `pnpm build`, `pnpm build:app`.
- Smoke test de `pnpm dev:app` y del MCP Tauri invocado mediante `pnpm exec mcp-server-tauri`.
- En CI, instalar con `--frozen-lockfile`, compilar para todos los targets de release y verificar que las acciones estén fijadas por SHA.
- Revisar `git diff` de manifiestos/lockfiles, `pnpm why decode-uri-component`, cambios transitivos y licencias antes de mergear.

## Referencias de implementación

- pnpm supply-chain security: https://pnpm.io/supply-chain-security
- pnpm settings: https://pnpm.io/settings
- Advisory Vitest: https://github.com/advisories/GHSA-82fw-gwwq-j7x9
- Advisory decode-uri-component: https://github.com/advisories/GHSA-vcc3-ghjq-m6fr

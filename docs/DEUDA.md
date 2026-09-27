# Marvis — deuda pendiente

**Fecha del inventario:** 2026-09-27.

Este archivo no duplica el plan: la secuencia de ejecución sigue en [PLAN_PENDIENTE.md](PLAN_PENDIENTE.md), el alcance normativo en [MARVIS_DELIVERY_PLAN_v0.4.md](MARVIS_DELIVERY_PLAN_v0.4.md), y la evidencia ya obtenida en [MVP_VALIDATION.md](MVP_VALIDATION.md) y [D2_RUNBOOK.md](D2_RUNBOOK.md). Aquí se concentra **todo lo que falta**, para poder verlo de un vistazo y borrarlo al cerrarlo.

**Regla de lectura:** nada se da por hecho sin evidencia. Lo que solo tiene prueba unitaria se dice así; lo que necesita la app nativa queda como `NO EJECUTADO`, nunca como aprobado. Cuando un ítem se cierre, se borra de aquí y se registra en la auditoría.

---

## 1. Fotografía

| Métrica                | Valor                                                                                                            |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Pruebas frontend       | 201 (`pnpm test:frontend`)                                                                                       |
| Pruebas Rust           | 132 (`pnpm test:rust`)                                                                                           |
| Pruebas de seguridad   | 1 frontend (`SEC-`) + 14 Rust (`sec_`) con `pnpm test:security`                                                  |
| D1-01…D1-20            | **10 PASS automatizado · 8 PARCIAL · 2 SIN VERIFICAR**                                                           |
| SEC-01…SEC-07          | **4 PASS automatizado · 3 PARCIAL**                                                                              |
| D2-01…D2-15            | **5 pasaron · 1 no ejecutado (D2-13) · 2 bloqueados por API (D2-07/08) · 7 sin ensayar (D2-03 sin implementar)** |
| D3-01…D3-10            | **0 empezados**                                                                                                  |
| Build                  | `pnpm build:app` arma el DMG; **sin firmar ni notarizar**                                                        |
| Chequeos locales       | `fmt:check`, `lint` (2 avisos preexistentes), `lint:rust`, `typecheck`, `test`, `test:security` en verde         |
| Plan de refactor de UI | **8 de 8 fases ejecutadas**; falta el recorrido manual nativo (ver §5)                                           |

---

## 2. P0 — Entrega 1 (_The Workstation_): lo que falta

### 2.1 Decisiones sin cerrar

| Q   | Tema                                   | Deuda                                                                                                                                                                                                                                  |
| --- | -------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Q11 | Repo sin remoto o sin rama reconocible | Regla «usar `origin/HEAD`; si no es fiable, pedir la rama una vez y persistirla» tiene implementación y pruebas parciales. **Falta validar el prompt y su supervivencia tras reiniciar**, y cubrir repo sin remoto, unborn y detached. |
| Q7  | Submódulos                             | Tratamiento **provisional sin cerrar**: abrir explícitamente como repo independiente con aviso; worktrees enlazados desde submódulo **no demostrados**. No atribuir soporte no probado.                                                |
| Q8  | Repos bare                             | Rechazo explícito, **provisional sin cerrar**. Confirmar antes de dar por cerrada la resolución de rutas.                                                                                                                              |
| Q6  | Rama al quitar un worktree             | Decisión «conservar rama por defecto; borrar solo por elección expresa» está implementada y probada, pero **falta registrarla como decisión estable**.                                                                                 |
| Q12 | Definición de «actividad» concurrente  | **Sin definir**. No presentar un editor externo como activo si Marvis no puede observarlo.                                                                                                                                             |
| Q9  | Linux                                  | Sin decidir si validar Tauri/PTY en Linux antes de D2 o quedarse en macOS.                                                                                                                                                             |

### 2.2 Seguridad

| Caso   | Deuda                                                                                                                                                                                                                              |
| ------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| SEC-01 | v0.4 exige **canonicalizar** rutas con `..`; hoy se **rechazan**. Decidir si la especificación acepta el rechazo seguro o se adaptan implementación y pruebas. Containment y symlinks ya están cubiertos.                          |
| SEC-03 | Falta cubrir nombres de archivo con comillas, `;`, `$()` y saltos de línea como **un solo argumento** en el lanzamiento de _shells_ (Git y editores sí están cubiertos). Sin ejecución genérica desde Vue.                         |
| SEC-06 | **Falta la matriz negativa de pertenencia**: IDs de repo/checkout/sesión desconocidos o ajenos por familia de comandos, con pruebas de autorización en Rust. Si «caller» implica aislamiento entre ventanas, definir la identidad. |

### 2.3 Funciones D1 incompletas

- **Tipos de sesión `server` y `custom`:** v0.4 los exige en D1; hoy **solo existen presets de shell y Neovim**. Falta diseñar configuración y lanzamiento seguros, asociados al checkout, con argumentos separados, ciclo de vida y salida visible, sin aceptar ejecutable y argumentos arbitrarios desde IPC. O, si se recorta D1, dejar constancia explícita del cambio a v0.4 — **no fingir que están entregadas**.
- **Ligaduras reales:** se mantiene `FiraCode Nerd Font Mono`, 16 px, altura 1,2, pero **no hay ninguna vía implementada que forme ligaduras** en el WebView. Falta evaluar un renderer/addon viable para WebKit y demostrar visualmente `!= !== == === => -> <- <= >= && || ::` en la ventana Tauri. CSS por sí solo no es evidencia.
- **Correcciones de UX/lógica** que salgan de los recorridos nativos, conservando lectura limitada de archivos, Markdown sanitizado, ramas explícitas, protección al borrar worktrees y restauración sin PTYs ficticios.

### 2.4 Pruebas nativas D1 — todas `NO EJECUTADO`

Ninguna de estas baterías se ha corrido en la app macOS real; `terminal-spike/` y el navegador no cuentan.

| #   | Batería                           | Casos                        | Qué falta medir                                                                                                                                                                                                                                              |
| --- | --------------------------------- | ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1   | Puerta del spike terminal         | D1-07/08/18                  | `portable-pty → Channel → WebView`: `yes` 30 s con **payload con checksum** (no solo `yes`), `rg`, `cargo build`, `pnpm install`, `nvim`, `htop`, resize rápido, 5 sesiones; pérdida de bytes, latencia, caudal, CPU/RAM, frame time; **CPU en reposo ≈ 0**. |
| 2   | Primer recorrido de punta a punta | D1-05/09                     | Crear worktree desde la acción del repo (base `main`, destino `.worktrees/<name>`), enfocar, lanzar shell con `cwd` correcto; editar desde terminal y ver Files/Changes sin refresh; carpeta plain sin funciones Git.                                        |
| 3   | Carga y visibilidad               | D1-11/16                     | Diff de 4.000+ líneas navegable **midiendo tiempos** (no infiriendo por la virtualización); terminal y editor editando el mismo checkout con señal de actividad concurrente (Q12).                                                                           |
| 4   | Fallos y restauración             | D1-14/17/19                  | Borrar un worktree fuera de Marvis → Missing/Locate/Close; cerrar y reabrir conservando repo, orden, selección y geometría sin restaurar PTYs; repo sin `main` → error.                                                                                      |
| 5   | Editores                          | D1-13                        | Abrir archivo/línea exacta en Zed y Neovim desde la app **y desde la build empaquetada**; documentar la ruta si alguno no está instalado.                                                                                                                    |
| 6   | Reejecución en contexto nativo    | D1-01…04, 06, 10, 12, 15, 20 | Las pruebas unitarias no demuestran el ciclo UI→IPC→OS; reejecutar según riesgo.                                                                                                                                                                             |

**Presupuestos que nadie ha medido aún:** Git <100 ms (<5k), <300 ms (5k–50k), <1 s (50k–200k) y siempre asíncrono; arranque en frío <2 s; cálido <1 s objetivo; 60 fps objetivo; cambio de checkout <100 ms percibidos. Si no se alcanzan, **registrar la desviación y el plan de corrección**, no mover el criterio.

### 2.5 Empaquetado y salida

- **Firma y notarización:** la build anterior produjo un DMG **sin firmar**. `tauri.conf.json` declara hardened runtime y destino DMG, pero eso **no es** un artefacto firmado. Configurar credenciales y verificar apertura en otra máquina antes de publicar; no bloquear el desarrollo local por carecer de ellas, pero **no publicar como firmado**.
- **Chunk de 1,08 MB:** `ChangesPane` pesa 1,083,81 kB minificado (331,39 kB gzip). Investigar impacto real en arranque e interacción y dividirlo si perjudica. El aviso por sí solo no es un fallo funcional.
- **Verificación de apertura en otra máquina** y ruta de actualización: sin empezar.

---

## 3. P1 — Entrega 2 (_Agent Loop_): lo que falta

### 3.1 Hecho, pero sin marcar ni registrar

- **Q2 — matriz del Spike 2:** la matriz observado/esperado con versión y capacidades reales **ya está escrita** en `MVP_VALIDATION.md` §Spike 2. Falta **tildar** el primer punto de P1.1 en el plan y darlo por cerrado.
- **El loop con dos agentes** ya está validado (`two_checkouts_run_the_review_loop` + runbook). Falta **tildar** el último punto de P1.3 y cerrar la tarea.
- **Q4** quedó cerrada el 2026-09-26 (aviso + enviar/encolar/ahora no, ronda encolada con mensaje almacenado, flush sin duplicar). Falta reflejarlo en el checklist de P1.1.

### 3.2 Sin implementar

- **`Follow Agent` (D2-03): no hay código.** No existe rastreo del archivo activo del agente, ni pausa por selección manual con oferta de reanudar, ni «OFF por defecto durante review». Es el único caso D2 cuyo hueco es de implementación y no de ensayo.
- **Flujo «nuevo worktree → abrir checkout → iniciar OpenCode» (D2-01)** con su variante desde un checkout Git existente.
- **Estados del sidebar con checkout no enfocado (D2-02):** `agentAttention` cubre `busy`/`blocked`, pero falta el estado de **error** en rojo y el ensayo de que dos agentes simultáneos no se pisan, más la barra global reflejando pendientes.
- **Mensajes, permisos, cancelación, fallo y reinicio «con capacidades reales del bridge»** — ver §3.4: dos de ellos no existen en la versión instalada.

### 3.3 Casos D2 sin ensayar

| Caso  | Qué pide                                                         | Estado              |
| ----- | ---------------------------------------------------------------- | ------------------- |
| D2-01 | Una acción crea worktree, lo abre y arranca el agente            | NO EJECUTADO        |
| D2-02 | Estado del agente visible en sidebar con checkout no enfocado    | NO EJECUTADO        |
| D2-03 | Follow Agent sigue el archivo que se está editando               | **SIN IMPLEMENTAR** |
| D2-06 | Dos agentes; el que termina primero muestra su estado            | NO EJECUTADO        |
| D2-11 | Matar el proceso OpenCode → estado de error y oferta de reinicio | NO EJECUTADO        |
| D2-12 | OpenCode ausente de PATH → la app lo explica y sigue usable      | NO EJECUTADO        |
| D2-13 | Reiniciar Marvis con dos agentes vivos y drafts sin enviar       | NO EJECUTADO        |
| D2-14 | Agente y terminal editando el mismo checkout → señal concurrente | NO EJECUTADO        |

Pasaron y están documentados en `D2_RUNBOOK.md`: **D2-04, D2-05, D2-09, D2-10, D2-15**.

### 3.4 Bloqueados por la versión de OpenCode (2.0.18) — no por Marvis

- **D2-07 — permisos:** el evento `permission.asked` llega y tiene toda la información para renderizar «permitir una vez / siempre / denegar», pero **ninguna ruta de respuesta existe** (todas 404). Un permiso sin responder **cuelga el turno indefinidamente** (180 s y counting: sin `outcome`, sin `time.idle`).
- **D2-08 — cancelación:** no hay ruta de abortar (404 en `abort`, `cancel`, `stop`, `interrupt`, `kill`). No se puede detener un turno por HTTP.
- **Mitigación acordada (best-effort):** que Marvis sea quien fije la política de permisos del agente para el turno de review mediante un `opencode.json` con alcance de checkout que permita `bash`/`edit` dentro del checkout. Un turno que se atasca en un permiso sin respuesta es peor que uno permitido.
- **Q10** queda en consecuencia: Marvis **no da ninguna garantía de revertir cambios parciales**, solo hacerlos visibles en el diff — y hoy ni siquiera puede interrumpir.

### 3.5 Riesgos operativos conocidos (limitaciones aceptadas, no bugs)

1. **Un review ambiguo deja el turno colgado para siempre.** Si el modelo se queda deliberando, `time.idle` nunca se escribe aunque el archivo ya haya cambiado y el mensaje esté `completed`. Sin cancelación, la única respuesta es esperar. **La instrucción del review tiene que ser inequívoca** (p. ej. `replace the whole file with exactly: …`).
2. **Una ronda encolada contra un agente bloqueado por permiso no se envía nunca**, porque ese turno no termina. Queda visible en el contador de rondas sin cerrar.
3. **El fin de turno se observa, no se anuncia.** No hay evento `session.idle` ni `session.execution.completed` en 2.0.18; solo `time.idle` en el objeto de sesión. Si el cliente no vio arrancar el turno, no puede saber que terminó.
4. **Un stream SSE nuevo no reproduce historial** y no hay token de reanudación: un cliente que pierde eventos solo se recupere releyendo los mensajes de la sesión y el diff.
5. **`GET /api/session` devuelve sesiones de todos los directorios** y `location` cambia de sitio según la ruta. El alcance hay que comprobarlo en ambos sitios y filtrar la lista, no confiar en el `id`.

---

## 4. P2 — Entrega 3 (_Depth_): nada empezado

| #   | Trabajo                                                                                                                                                                                    | Casos       |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------- |
| 1   | **Reanchoring / Q5:** contexto + hash, nueva posición solo con alta confianza, origen y destino visibles; baja confianza → `outdated`, nunca asociación con código ajeno                   | D3-01       |
| 2   | **Previews ricos:** PNG/JPEG/WebP/GIF con dimensiones, fit/100 %/zoom y apertura externa sin bloquear la UI; SVG sin ejecutar scripts; JSON plegable y YAML formateado                     | D3-02…D3-04 |
| 3   | **Neovim RPC:** descubrir socket por checkout, reutilizar instancia, saltar a línea exacta; socket ausente/obsoleto → arranque nuevo con mensaje                                           | D3-05…D3-06 |
| 4   | **Renderer:** spike libghostty frente a WebView con los mismos datos (caudal, latencia, CPU, memoria, frame time, resize) y decisión publicada. Aquí se decide también lo de las ligaduras | D3-07       |
| 5   | **Diff:** lado a lado y stage/unstage por hunk con salvaguardas para archivos cambiantes, binarios y cambios concurrentes                                                                  | —           |
| 6   | **Ergonomía:** wide/medium/narrow, navegación completa por teclado, temas claro/oscuro del sistema y tamaño configurable de terminal/inspector                                             | D3-08…D3-09 |
| 7   | **Rendimiento y distribución:** suite de benchmarks reproducible con resultados guardados y detección de regresiones; memoria acotada, onboarding, firmado, ruta de actualización          | D3-10       |

No adelantar el reanclaje sin diffs reales de agente, y no empezar el renderer hasta tener los datos del spike.

---

## 5. Refactor de UI hacia el mockup (corriente paralela)

Plan propio en [plan_refactor_ui.md](plan_refactor_ui.md): **no sustituye** a v0.4 ni a `PLAN_PENDIENTE.md`, pero está aprobado. **Las 8 fases están ejecutadas**; lo que queda de esta corriente es la deuda que la ejecución destapó y la que falta cerrar a mano.

- **8 de 8 fases ejecutadas.** Fase 0 (tokens y ventana) → 1 (titlebar/splitter) → 2 (sidebar) → 3 (Files/Changes) → 4 (main unificado) → 5 (toasts) → 6 (notas y envío a opencode) → 7 (limpieza).
- **Cierre de fase = el checklist §8 del plan:** los 8 puntos de funcionalidad más `fmt:check`, `lint`, `lint:rust`, `typecheck`, `test:frontend`, `test:rust`, `test:security`, `build:app` y el recorrido manual en la app nativa (worktree, terminal, `nvim`, diff, diff completo, comentar, enviar al agente, `.md`, error de repo, resize angosto). **Las 9 verificaciones automáticas están en verde tras la fase 7**; **el recorrido manual en la app nativa sigue `NO EJECUTADO`** y es lo único que impide dar la corriente por cerrada.
- **Deuda que dejó la fase 7 (limpieza):**
  - **El dato `viewed` sigue persistido en Rust y ya no lo lee nadie** (~34 ocurrencias en `services/workspace.rs`, `services/git.rs`, `commands/git.rs`, `main.rs`, `persistence/mod.rs`: tabla `viewed_files`, `mark_file_viewed`, `viewed_files`, y los comandos `git_viewed_files` / `git_mark_viewed`). La decisión E.7 sacó la lectura del frontend; **borrar la capa de persistencia es una migración de datos** que este refactor no estaba autorizado a hacer. Hay que decidir: recuperar la capacidad o migrar la tabla fuera.
  - **Cuatro comandos Rust quedaron sin `invoke` desde el WebView** al borrar sus wrappers del frontend: `git_viewed_files`, `git_mark_viewed`, `agent_prompt` y `review_notes_mark_sent`. Los servicios que hay detrás siguen vivos (`agents.prompt` lo usa el envío de rondas; `review::mark_sent` lo usa la capa de review), pero **los comandos registrados no tienen consumidor**. Decidir si se bajan del `invoke_handler` o si se les vuelve a dar entrada de UI. `locate_missing_checkout` se suma por la decisión C.1, que sacó la reubicación de la UI: no tiene consumidor por decisión, no por descuido.
  - **`src/components/ui/button/Button.vue`** es un botón escrito a mano que quedó en la carpeta que usa la convención de shadcn, ya sin `components.json` ni CLI. Vive, pero está fuera de lugar.
- **Cerrado después de la fase 7 — el caso `D1-14/17/19` (checkout cuyo directorio desaparece):**
  - **Con la app abierta:** la fila queda **deshabilitada** (atenuada, `aria-disabled`, no seleccionable, sin items ni `New terminal`) y su **única** acción es un `XIcon` que la quita de la lista, con confirmación que nombra el alcance (el repo entero si es la raíz o una carpeta plain, la entrada sola si es un worktree). No borra archivos: para un worktree git ejecuta `git worktree prune --expire now` con `delete_branch: false`, que solo limpia la metadata de `.git/worktrees/` de un directorio que ya no existe.
  - **Al reabrir:** `Database::prune_missing_checkouts` borra todo checkout cuyo `canonical_path` no sea un directorio, y todo repo que se quede sin checkouts; si el checkout activo caía, la selección pasa al primario sobreviviente y se limpia `ACTIVE_SESSION`. Es una función sola y el orden importa: corre **después** de la reconciliación git, porque `reconcile_git_repo` reinserta las entradas podables que `git worktree list` sigue reportando. **Riesgo aceptado y documentado en el doc-comment:** un volumen no montado falla el mismo `is_dir()`, así que reabrir con el volumen desconectado descarta los worktrees registrados en él. Para revertir basta con quitar la llamada a `prune_missing_checkouts` de `services::workspace::restore`.
  - El guard `if kind == "git" && !is_primary` de `persistence::close_missing_checkout` se sakó porque era **código muerto**: el servicio intercepta los worktrees git faltantes y los manda a `worktree::remove`, que ya corría la ruta segura. El caso `D1` de §2.4 fila 4 vuelve a ser ejercitable.
- **Decisiones que se aceptaron con default quemado** (§9 del plan) y que **nadie ha ratified**: JetBrains Mono no bundleada, chevron del crumb con dropdown, doble-click-to-zoom sí, punto de estado solo por color, click en workdir muestra el item activo, el badge de Changes cuenta archivos, **E.7 (se cae `Viewed`)**, se siguen guardando los anchos, no se reescribe el copy, `↗ Neovim` sí, el copy del diff completo es `All changes`. Cada una está en el código y ninguna está escrita como decisión estable.
- **Riesgo de rework superado:** se reescribieron o borraron los tests **por fase, no al final**. HOY son **205 frontend / 138 Rust** (el §6 del plan parte de «64 + 80» y ya no describe el estado real).
- **Doble sistema de tokens:** resuelto; `--surface-*` no existe (la fase 0 lo borró a mano, `pnpm lint` no lo caza).

**Inventario de tests, con el cierre del caso `isMissing`:** 205 frontend (`pnpm test:frontend`) y 138 Rust (`pnpm test:rust`), más 1 frontend `SEC-` y 14 Rust `sec_` con `pnpm test:security`.

## 6. Deuda de documento

| Dónde                                 | Qué sobra o está mal                                                                                                                                                                                                   |
| ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `PLAN_PENDIENTE.md` §1                | Dice «9 casos con cobertura automatizada, 9 parciales y 2 sin verificar» y «64 pruebas frontend y 80 Rust». **Desactualizado**: hoy son **10/8/2** y **205/138**. Es histórico, pero conviene fecharlo o actualizarlo. |
| `MVP_VALIDATION.md` §Verification run | La tabla de resultados también quedó en **64/80** y en **8** pruebas `sec_`. Hoy son 201/132 y **14**. Debe reejecutarse y regrabarse antes de dar por buena la auditoría.                                             |
| `PLAN_PENDIENTE.md` §3                | Los checkboxes de **P1.1, P1.2 y P1.3** siguen sin marcar pese a que buena parte del trabajo ya existe y está probado (ver §3.1). Mientras no se marquen, la puerta P1→P2 no se puede declarar.                        |
| `docs/`                               | `GIT_SPIKE_FINDINGS.md` **no tiene ninguna referencia cruzada**. Documenta `git-spike/`, que todavía existe en el repo. Candidato a borrar o a enlazar desde el plan; ahora mismo flota.                               |
| Requisitos v0.2                       | Renombrado a `MARVIS_REQUIREMENTS.md` (era `.old`). **Comprobar de vez en cuando** que v0.4 sigue refiriéndose a ellos como compañía y no hay una v0.3 suelta.                                                         |

---

## 7. Deuda de infraestructura

- **Los tests del agente no corren en CI.** `a_round_marker_reaches_the_real_session`, `a_turn_is_observable_through_the_idle_time`, `a_queued_round_goes_out_when_flushed` y `two_checkouts_run_the_review_loop` necesitan `MARVIS_AGENT_BRIDGE=1`, dos repos locales y un binario `opencode` real. Se verifican a mano y **se pierden si nadie los ejecuta**. Falta decidir: o un job de CI con un `opencode serve` de fixture, o registrar explícitamente que son una verificación local.
- **CI** (`.github/workflows/ci.yml`, `macos-latest`) cubre `fmt:check`, `lint`, `lint:rust`, `typecheck`, `test`, `test:security` y `build:app`. **No** valida artefacto firmado ni ejecuta ninguna de las baterías nativas de §2.4.
- **Sin ruta de actualización** y sin verificación de apertura en otra máquina.
- **Bundle sin optimizar:** `ChangesPane` 1,08 MB; no se ha intentado ningún split ni code-splitting.

---

## 8. Orden sugerido

1. **Cerrar lo documental que ya está hecho** (barato y desbloquea la puerta): tildar P1.1 y P1.3, registrar Q3, Q4 y Q10, actualizar las cifras de `PLAN_PENDIENTE.md` y `MVP_VALIDATION.md` §Verification run.
2. **P1 hasta la puerta P1 → P2:** `Follow Agent` (D2-03), flujo D2-01, estados D2-02, y ensayar D2-06/11/12/14. Dejar D2-13 para cuando se pueda reiniciar la app empaquetada con dos agentes vivos.
3. **P0.3 — pruebas nativas D1:** es donde más casos están en `PARCIAL`/`NOT VERIFICAR`; sin esto no se puede cerrar D1 ni declarar la puerta P0 → P1.
4. **P0.1 y P0.2:** cerrar SEC-01/03/06 y Q7/Q8/Q11/Q12; decidir `server`/`custom` y atacar las ligaduras.
5. **P0.4** — firma, notarización, chunk y Q9.
6. **Refactor de UI (corriente paralela, §5):** puede avanzar en paralelo a P0/P1 porque es de forma, no de alcance, pero **las fases 6 y 7 tocan el envío de review y la palette**, así que conviene encajarlas con el cierre de P1 y no después.
7. **P2** — solo cuando D2 esté cerrado y haya diffs reales de agente para el reanclaje.

**Criterio de cierre global:** una tarea sin evidencia sigue abierta. Si algo queda sin verificar por un límite de plataforma (como D2-07/D2-08), se documenta como limitación best-effort — nunca se marca como cumplida.

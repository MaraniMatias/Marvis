# Runbook de aceptación — Entrega 2 (D2-04, D2-05, D2-09, D2-10, D2-13, D2-15)

Ejecutado el **2026-09-26** contra el `opencode` **v2.0.18** instalado, con **dos agentes
vivos en dos checkouts distintos**. Cada caso queda registrado como **pasó**, **falló** o
**no ejecutado**, con la evidencia concreta que lo sostiene y el paso exacto que la produjo.

Este documento no sustituye a `MVP_VALIDATION.md`: ahí está la matriz del spike (qué
expone o no la interfaz de opencode). Aquí está si el loop de review de Marvis cumple.

---

## Entorno

Dos repos git independientes, cada uno con su propio `opencode.json` y su propio servidor:

```text
…/T/opencode/bridge/one   ← primer checkout
…/T/opencode/bridge/two   ← segundo checkout
```

Reproducción completa (los cuatro tests gateados, ninguno corre en CI):

```bash
export PATH="$HOME/.opencode/bin:$PATH"
export MARVIS_AGENT_BRIDGE=1
export MARVIS_AGENT_BRIDGE_DIR=…/T/opencode/bridge/one
export MARVIS_AGENT_BRIDGE_OTHER_DIR=…/T/opencode/bridge/two
cargo test --manifest-path src-tauri/Cargo.toml
```

Resultado de esa corrida: **119 passed, 0 failed**, incluidos los cuatro gateados
(`bridge_talks_to_a_real_server`, `a_round_marker_reaches_the_real_session`,
`a_turn_is_observable_through_the_idle_time`, `two_checkouts_run_the_review_loop`).

El proceso de aceptación corrió **dos `opencode serve` simultáneos**, uno por checkout: la
línea de base de Marvis es «un servidor por checkout», y todo el resto del loop se midió
sobre esa configuración.

---

## Resumen de veredictos

| Caso  | Veredicto        | Evidencia decisiva                                                                                        |
| ----- | ---------------- | --------------------------------------------------------------------------------------------------------- |
| D2-04 | **pasó**         | `review_drafts_across_two_files_survive_a_database_restart`                                               |
| D2-05 | **pasó**         | `two_checkouts_run_the_review_loop` + `a_round_marker_reaches_the_real_session`                           |
| D2-09 | **pasó**         | `two_checkouts_run_the_review_loop` + `DocumentPane` «judges each note…» y «reports the anchor text…»     |
| D2-10 | **pasó**         | `a_drifted_anchor_is_marked_outdated_and_only_the_user_clears_it` + `ChangesPane` «holds outdated notes…» |
| D2-13 | **no ejecutado** | Faltó el paso definitorio: reiniciar con dos agentes vivos y comprobar el arranque                        |
| D2-15 | **pasó**         | `two_checkouts_run_the_review_loop`                                                                       |

Ninguno falló. Uno no se ejecutó.

---

## D2-04 — Tres comentarios de línea en dos archivos persisten tras el reinicio como borradores

| #   | Paso                                                                                   | Resultado |
| --- | -------------------------------------------------------------------------------------- | --------- |
| 1   | Crear tres notas: dos en `src/main.rs` (líneas 4 y 12) y una en `src/lib.rs` (línea 7) | pasó      |
| 2   | Cerrar y reabrir la base de datos                                                      | pasó      |
| 3   | Leer por el manejador **reabierto**, no por el que escribió                            | pasó      |
| 4   | Las tres siguen en `draft`, sin `outdated`, con `code`, `code_hash` y rango intactos   | pasó      |

**Evidencia:** `review_drafts_across_two_files_survive_a_database_restart`
(`src-tauri/src/persistence/mod.rs`).

**Notas de ejecución:**

- El reinicio se hace reabriendo `Database::open` sobre el mismo archivo, que es lo que un
  reinicio de Marvis hace con las notas. Las notas se leen por el manejador reabierto a
  propósito: lo que está en memoria no vale como persistencia.
- Una nota queda **sin `line_end`** (rango abierto) y se afirma explícitamente: es la que
  tiene que seguir apuntando al código correcto después del reinicio.
- El alcance real de «reinicio de Marvis» (ventana, layout, repos) se cubre en D2-13.

**Veredicto: pasó.**

---

## D2-05 — «Enviar review» entrega todos los drafts al agente como un solo mensaje

| #   | Paso                                                                                        | Resultado |
| --- | ------------------------------------------------------------------------------------------- | --------- |
| 1   | Escribir dos archivos a revisar en el primer checkout                                       | pasó      |
| 2   | Armar el review con un comentario por archivo                                               | pasó      |
| 3   | Un solo `prompt` con el prefijo de instrucciones, el **marker** y ambas secciones           | pasó      |
| 4   | La sesión responde **ocupa y luego inactiva** (el turno corrió)                             | pasó      |
| 5   | El marker aparece en la transcripción de la sesión                                          | pasó      |
| 6   | Ambos archivos cambiaron: el agente trabajó sobre el lote completo                          | pasó      |
| 7   | Segundo checkout: su servidor existe, su sesión no corre, y **no ve** la sesión del primero | pasó      |

**Evidencia:**

- `two_checkouts_run_the_review_loop` — construye el prompt con `build_round_prompt`,
  afirma que lleva el marker **y** los dos archivos, llama `prompt` **una sola vez**,
  espera `idle_at`, y después afirma que el marker está en la transcripción y que los dos
  archivos cambiaron.
- `a_round_marker_reaches_the_real_session` — afirma el marker **ausente antes** de enviar
  y **presente después**. La mitad negativa importa tanto como la positiva: una lectura de
  transcripción que fallara hacia «siempre coincide» convertiría el dedup en una pérdida
  silenciosa de reviews.
- `a_round_is_recorded_before_it_is_sent_and_carries_its_marker` — el estado
  `dispatching` con el marker ya guardado **antes** de llamar al agente.

**Notas de ejecución:**

- Cada instrucción del review es deliberadamente inequívoca («replace the whole file with
  exactly: …»). Encontré por qué: con un comentario ambiguo el modelo se queda deliberando
  y **el turno no cierra** — ver «Hallazgos» más abajo.
- Tres corridas consecutivas del loop: 22,7 s / 7,8 s / 13,4 s, las tres en verde.

**Veredicto: pasó.**

---

## D2-09 — Tras un review enviado y un turno nuevo, cada comentario muestra si sus líneas cambiaron y puede marcarse resuelto

| #   | Paso                                                                                   | Resultado |
| --- | -------------------------------------------------------------------------------------- | --------- |
| 1   | Enviar el review y que el turno **termine** (señal de fin de turno observable)         | pasó      |
| 2   | El diff se recarga con el cambio de estado Git                                         | pasó      |
| 3   | El texto actual de cada ancla se manda a `verifyAnchors` con la línea vigente          | pasó      |
| 4   | Cada comentario muestra su veredicto: `unchanged` y `missing` se ofrecen, `changed` no | pasó      |
| 5   | Marcar resuelto de un clic funciona y solo donde el diff lo puede respaldar            | pasó      |

**Evidencia:**

- Paso 1: `two_checkouts_run_the_review_loop` espera el `idle_at` con una cola de 180 s y
  falla con el último estado leído si no llega. `a_turn_is_observable_through_the_idle_time`
  fija además la ambigüedad del campo.
- Pasos 2 y 3: `DocumentPane` «reports the anchor text of visible notes and holds drifted
  ones back» afirma que `verifyAnchors` se llama con **`currentCode` de la línea vigente**,
  y «retains the selected diff on status refresh…» cubre la recarga ante un cambio de Git.
- Pasos 4 y 5: `DocumentPane` «judges each note from the diff and only offers resolve when
  it can back it» — en modo hunk afirma que aparecen _«The line this note points at is
  unchanged.»_ y _«… no longer exists.»_, y que los botones resolvibles son exactamente
  `['Mark note on line 1 as resolved', 'Mark note on line 40 as resolved']`.
- El cierre de la ronda: `App.vue` observa `turnsCompleted` y llama `ackFinishedTurn()`.

**Notas de ejecución:**

- El veredicto lo da **el diff**, que es el único que sabe qué líneas existen. `outdated`
  es la comparación de hash del backend y manda; si la línea no está y el diff entero está
  en memoria, es `missing`; si el diff es grande y todavía no cargó esa línea es `unknown`,
  que **no** se puede afirmar como eliminada.
- Una nota `sent` cuya línea desapareció **no tiene fila** donde colgarse en el diff, así
  que se lista aparte. Sin eso, el caso en el que más probablemente el agente actuó sería
  justo el que el usuario no podría ver ni resolver.

**Veredicto: pasó.** Reserva honesta: no se observó el veredicto en la **app corriendo**,
solo a nivel de componente y de servicio; el mecanismo está cubierto, la observación visual
en una ventana real no.

---

## D2-10 — Un comentario cuya línea cambió queda `outdated` y nunca se adjunta a código ajeno

| #   | Paso                                                                                   | Resultado |
| --- | -------------------------------------------------------------------------------------- | --------- |
| 1   | Nota con su hash de ancla capturado al crearla                                         | pasó      |
| 2   | La línea cambia (la reescribe el agente en el loop) y el hash ya no coincide           | pasó      |
| 3   | `verify_note_anchors` la marca `outdated`                                              | pasó      |
| 4   | Solo el usuario la puede limpiar (`clear_outdated` no la toca si no está marcada)      | pasó      |
| 5   | Editar el **contenido** del comentario no le borra la marca                            | pasó      |
| 6   | Queda **fuera del lote por defecto**; un checkbox la reincorpora                       | pasó      |
| 7   | La migración v8 rellena `code_hash` de las notas existentes sin marcar nada como vieja | pasó      |

**Evidencia:**

- Paso 2: `two_checkouts_run_the_review_loop` afirma que
  `review_anchor_hash(original) != review_anchor_hash(después del turno)` sobre un archivo
  que el agente reescribió de verdad.
- Pasos 3-5: `a_drifted_anchor_is_marked_outdated_and_only_the_user_clears_it`,
  `editing_a_note_leaves_its_outdated_mark_alone`.
- Paso 6: `ChangesPane` «holds outdated notes out of the round until the user opts back in»
  — _«1 outdated note left out»_ y el `[data-testid="include-outdated"]` que la reincorpora.
- Paso 7: `migrating_to_v8_backfills_anchor_hashes_for_existing_notes`.
- Alcance: `sec_08_anchor_checks_cannot_target_a_foreign_checkout_or_path`.

**Notas de ejecución:**

- Solo se hashea **la primera línea** de la ancla. Un comentario multi-línea cuyo bloque
  estuvo parcialmente cargado no puede parecer desplazado nunca.
- FNV-1a: detecta deriva accidental, no es una frontera de seguridad.

**Veredicto: pasó.**

---

## D2-13 — Reiniciar Marvis con dos agentes activos y drafts sin enviar; se restauran repos, layout y drafts

| #   | Paso                                                                     | Resultado        |
| --- | ------------------------------------------------------------------------ | ---------------- |
| 1   | Los repos y su orden sobreviven el reinicio de la base                   | pasó             |
| 2   | El layout de terminales y la UI por checkout sobreviven                  | pasó             |
| 3   | Los drafts sobreviven (D2-04)                                            | pasó             |
| 4   | **Dos agentes activos al momento del reinicio**                          | **no ejecutado** |
| 5   | Tras el arranque, los dos servidores vuelven y las sesiones se recuperan | **no ejecutado** |

**Evidencia de los pasos 1-3:** `plain_repo_order_selection_and_identity_survive_restart_without_duplicates`,
`checkout_layouts_persist_and_heal_sessions_owned_by_another_checkout`,
`ui_state_round_trips_migrates_and_discards_invalid_saved_data`,
`review_drafts_across_two_files_survive_a_database_restart`.

**Por qué 4 y 5 no se ejecutaron:** el paso definitorio del caso es **reiniciar la app
con dos agentes corriendo**. Los tres primeros pasos son reinicios de la capa de
persistencia, y eso sí se corrió; el ciclo completo —ventana, dos `opencode serve` vivos,
arranque de nuevo— requiere la aplicación empaquetada y una sesión interactiva, y no se
corrió en este pase.

Esto **no** es un fallo: es evidencia faltante. El caso queda abierto hasta que alguien
cierre la app con dos agentes levantados y la vuelva a abrir.

**Veredicto: no ejecutado.**

---

## D2-15 — El loop completo cierra de punta a punta

Cadena: **worktree → agente → diff → review → envío → el agente continúa → review otra vez.**

| #   | Paso                                                                                  | Resultado |
| --- | ------------------------------------------------------------------------------------- | --------- |
| 1   | El checkout tiene su servidor y su sesión                                             | pasó      |
| 2   | El review sale como un mensaje con marker                                             | pasó      |
| 3   | El turno arranca y termina                                                            | pasó      |
| 4   | El árbol de trabajo se movió: el agente editó **los dos** archivos señalados          | pasó      |
| 5   | El hash de la ancla ya no coincide con la línea original                              | pasó      |
| 6   | El diff se recarga, `verifyAnchors` corre con la línea nueva y el comentario se juzga | pasó      |
| 7   | Marcar resuelto de un clic                                                            | pasó      |

**Evidencia:** pasos 1-5 en `two_checkouts_run_the_review_loop` (corrido dentro de la
línea de base de dos checkouts); pasos 6-7 en los tests de `DocumentPane` citados en
D2-09.

**Notas de ejecución:**

- El paso 1 usa los repos de prueba del runbook, que son `git init` simples, **no** un
  worktree creado por la acción de D2-01. La creación de worktrees es un caso aparte y
  **no forma parte de este runbook**.
- Los pasos 6 y 7 se verifican a nivel de componente, igual que en D2-09.

**Veredicto: pasó.**

---

## Lo que se intentó y no sirvió

Registrado para que no se repita.

**El review apuntando a un archivo que el modelo tenía que interpretar dejó el turno
colgado.** Con un comentario ambiguo —_«line 1: covered by the same message»_ sobre un
archivo que no era el objetivo— el modelo se quedaba deliberando y **`time.idle` nunca se
escribía**, aunque el archivo ya hubiera cambiado y el mensaje del asistente estuviera
`completed`. Marvis esperaría para siempre y mostraría el agente como «trabajando», y en
esta versión **no hay forma de cancelar el turno** (D2-08, 404).

La corrección del runbook fue trivial y es la lección: **un review debe dar instrucciones
inequívocas**. Reescrito a _«replace the whole file with exactly: …»_, el mismo loop pasó
tres veces seguidas.

---

## Hallazgos que condicionan el alcance

1. **D2-07 y D2-08 siguen bloqueados en 2.0.18.** No existe ruta HTTP de cancelación ni de
   respuesta a permisos (todas 404). Un permiso sin responder cuelga el turno
   indefinidamente. Documentado en `MVP_VALIDATION.md` §Spike 2 como limitación
   best-effort: la mitigación es que Marvis sea quien fije la política de permisos del
   agente para el turno de review.

2. **No hay evento de fin de turno.** El fin de turno se observa, no se anuncia: `idle_at`
   más un arranque de turno que el cliente haya visto. `busy` es derivado, no reportado,
   porque una sesión que nunca corrió tampoco tiene tiempo de inactividad.

3. **El `idle` puede no llegar aunque el trabajo ya se haya hecho.** Se observó al menos
   una vez un turno cuyo mensaje estaba `completed` y cuyo archivo ya había cambiado, sin
   `idle` escrito en 120 s. En Marvis eso se lee como «sigue trabajando». Es el modo de
   fallo que hay que vigilar en uso real, y por ahora la única respuesta posible es
   esperar: no hay cancelación.

---

## Cierre

De los seis casos de este pase: **cinco pasaron, ninguno falló, uno no se ejecutó**
(D2-13, por falta del reinicio con dos agentes vivos).

La puerta P1 → P2 de `PLAN_PENDIENTE.md` pide «D2-01…D2-15 documentados y
reproducibles». Este runbook cubre los seis casos que le corresponden a la Entrega 2 en
este pase; **D2-13 queda explícitamente abierto** y no debe darse por cubierto.

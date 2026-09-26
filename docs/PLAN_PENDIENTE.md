# Marvis — plan de trabajo pendiente

**Estado:** plan vigente de pendientes. **Plataforma inicial:** macOS. Sustituye a [MVP_plan.md](MVP_plan.md) como guía de ejecución; no sustituye el alcance ni los criterios de [MARVIS_DELIVERY_PLAN_v0.4.md](MARVIS_DELIVERY_PLAN_v0.4.md) y los requisitos v0.2. La evidencia ya recogida y los estados de cada criterio están en [MVP_VALIDATION.md](MVP_VALIDATION.md). Ante diferencias de alcance, prevalece v0.4; actualizar la auditoría conforme se cierren tareas.

## 1. Punto de partida y regla de cierre

La base, el recorrido MVP y gran parte de la Entrega 1 están implementados. En la última auditoría, D1-01…D1-20 tenían **9 casos con cobertura automatizada, 9 parciales y 2 sin verificar**; SEC-01…SEC-07 tenían **4 con cobertura automatizada y 3 parciales**. `pnpm test` pasó entonces con 64 pruebas frontend y 80 Rust. Son resultados de esa ejecución, **no una certificación de la aplicación nativa ni un estado garantizado para versiones posteriores**.

Orden de avance: **P0 — cerrar Entrega 1 → P1 — Entrega 2 / Agent Loop → P2 — Entrega 3 / Depth**. No considerar una entrega cerrada porque compile o porque pasen pruebas aisladas: registrar evidencia de sus casos D1/D2/D3 y SEC pertinentes, y probar los recorridos completos en macOS. Si una prueba revela un defecto, corregirlo antes de marcarla aprobada. No cambiar el modelo Repo → Checkout → Session, la referencia de diff contra la rama por defecto, ni el límite de privilegios del WebView.

Para cada bloque: implementar lo que falte, añadir pruebas repetibles, ejecutar `pnpm fmt:check`, `pnpm lint`, `pnpm lint:rust`, `pnpm typecheck`, `pnpm test`, `pnpm test:security` y `pnpm build:app` cuando proceda; registrar dispositivo/macOS, versión, comandos, resultados y límites en `MVP_VALIDATION.md` o una auditoría de entrega posterior. Los pasos que precisan GUI, firma o servicios externos se registran como **pendientes**, nunca como aprobados por un test de unidad.

## 2. P0 — terminar la Entrega 1

### P0.1 Decisiones y contrato de seguridad

- [x] **Q1:** resuelta para esta UI: usar `<repo>/.worktrees/<name>`, añadir la exclusión local sin reemplazar entradas y crear siempre desde `main`; error claro si `main` falta. No mover worktrees existentes por cambiar solo el documento.
- [ ] **Q11:** confirmar la regla «usar `origin/HEAD`; cuando no sea fiable, pedir la rama una vez y persistirla», incluyendo repo sin remoto, sin `main`, unborn y detached. La implementación tiene pruebas parciales: validar el prompt y su reutilización tras reiniciar.
- [ ] **Q7/Q8:** cerrar el tratamiento de submódulos (abrir explícitamente como repo independiente; worktrees enlazados no demostrados) y de repos bare (rechazo explícito), sin atribuir soporte no probado.
- [ ] **Q6/Q12:** registrar como decisiones estables, respectivamente, «conservar rama por defecto; borrar solo por elección expresa» y la definición observable de actividad concurrente. No presentar un editor externo como activo si Marvis no puede observarlo.
- [ ] **SEC-01:** v0.4 exige canonicalizar rutas con `..` antes de usarlas; hoy se rechazan. Decidir si la especificación acepta ese rechazo seguro o adaptar implementación y pruebas. Mantener comprobaciones de containment y symlinks.
- [ ] **SEC-03:** confirmar cobertura de nombres con espacios, comillas, punto y coma, `$()` y saltos de línea como **un solo argumento** en Git y editores; para shells, probar el caso aplicable al contrato real de lanzamiento, sin introducir una ejecución genérica desde Vue.
- [ ] **SEC-06:** construir matriz negativa de comandos por repo/checkout/sesión (IDs desconocidos o ajenos) y agregar pruebas de autorización en Rust. Conservar SEC-02/04/05/07 y volver a ejecutar las siete en CI. Si «caller» requiere aislamiento entre ventanas o usuarios, definir la identidad antes de afirmar que se cumple.

**Salida:** decisiones reflejadas en `MVP_DECISIONS_0.3.md` o su sucesor, SEC-01…SEC-07 automatizados y verificados en CI, sin capacidad genérica de proceso o filesystem para el WebView. El cierre documental de Q1 no sustituye la prueba funcional de worktrees.

### P0.2 Funciones D1 incompletas

- [ ] **Sesiones `server` y `custom`:** v0.4 las exige en D1; hoy solo hay presets backend de shell y Neovim. Diseñar configuración y lanzamiento seguros, asociados al checkout, con argumentos separados, ciclo de vida y error/salida visibles; no aceptar un ejecutable y argumentos arbitrarios desde IPC del WebView. Probar reinicio, cierre y sesiones simultáneas. Si se acuerda recortar D1, dejar constancia de un cambio explícito a v0.4; no fingir que ya están entregadas.
- [ ] **Ligaduras reales:** mantener `FiraCode Nerd Font Mono`, 16 px y altura 1,2. Evaluar un renderer/addon que pueda formar ligaduras en el WebView macOS sin romper Unicode, ANSI, selección, cursor ni PTY; implementar la opción efectiva y demostrar visualmente `!= !== == === => -> <- <= >= && || ::` en la ventana Tauri. La fuente está instalada en la máquina de desarrollo, pero eso y CSS por sí solos no prueban el requisito. Si la vía actual de xterm no sirve, registrar la evaluación y escoger un renderer viable; no marcar «ligaduras activas» sin evidencia.
- [ ] Corregir fallos de UX o lógica que surjan de los recorridos nativos siguientes, conservando lectura limitada de archivos, Markdown sanitizado, ramas explícitas, protección al borrar worktrees y restauración sin PTYs ficticios.

**Salida:** los cuatro tipos de sesión de D1 disponibles conforme al modelo de seguridad; tipografía y ligaduras comprobadas en WebKit; pruebas de regresión automatizadas para cada corrección.

### P0.3 Pruebas nativas e integración D1

Ejecutar en una app macOS real, no solo en `terminal-spike/` ni en un navegador ajeno a Tauri. Guardar resultado **pasó/falló/no ejecutado** por caso en la auditoría:

1. [ ] **Puerta del spike terminal / D1-07, D1-08, D1-18:** `portable-pty → Tauri Channel<ArrayBuffer> → WebView`; `yes` durante 30 s, payload exacto grande, `rg`, `cargo build`, `pnpm install`, `nvim`, `htop`, resize rápido y cinco sesiones. Medir pérdida de bytes (usar payload con checksum o secuencia, no solo `yes`), latencia, caudal, CPU/RAM, respuesta de UI, frame time, uso interactivo/pegado/ratón/alternate screen. Cinco sesiones en reposo deben dejar CPU próxima a cero; si falla, ajustar el pipeline o evaluar renderer antes de aprobar.
2. [ ] **Primer recorrido de punta a punta / D1-05, D1-09:** abrir repo y worktrees, crear uno desde la acción del repo (base `main`, destino `.worktrees/<name>`), enfocarlo y lanzar shell con cwd correcto; editar desde terminal y ver Files/Changes/status/diff actualizados sin refresh. Abrir carpeta plain y comprobar ausencia de funciones Git.
3. [ ] **Carga y visibilidad / D1-11, D1-16:** diff de 4.000+ líneas navegable sin congelación, solo filas visibles renderizadas; terminal y editor editando el mismo checkout muestran actividad concurrente según Q12. Medir tiempos en vez de inferir rendimiento por la virtualización del DOM.
4. [ ] **Fallos y restauración / D1-14, D1-17, D1-19:** eliminar un worktree fuera de Marvis y probar Missing/Locate/Close; cerrar y reabrir conservando repo, orden, selección y geometría, normalizando layouts antiguos a una sesión sin restaurar PTYs; repo sin `main` debe mostrar un error al crear worktree y nunca elegir otra rama.
5. [ ] **Editores / D1-13:** desde la app y desde la build empaquetada abrir archivo/línea exacta en Zed y Neovim; documentar ruta si uno no está instalado. Comprobar errores accionables.
6. [ ] Reejecutar en contexto nativo los casos D1-01…D1-04, 06, 10, 12, 15 y 20 ya cubiertos por pruebas automatizadas, según su riesgo; las pruebas existentes no demuestran por sí solas el ciclo UI→IPC→OS.

**Salida:** D1-01…D1-20 con evidencia, sin `PARTIAL`/`NOT VERIFIED` no justificados; presupuesto Git de v0.4 medido (<100 ms para <5k, <300 ms para 5k–50k, <1 s para 50k–200k; siempre asíncrono), startup frío <2 s, warm <1 s objetivo, 60 fps objetivo y cambio de checkout <100 ms percibidos donde haya caché. Si las cifras no se alcanzan, registrar desviación y plan de corrección en vez de cambiar el criterio silenciosamente.

### P0.4 Empaquetado y salida D1

- [ ] Revisar errores, estados vacíos y permisos reales de macOS; probar la build `pnpm build:app` y su DMG. La build anterior produjo un DMG **sin firma/notarización**; configurar credenciales y verificar firma, notarización y apertura en otra máquina cuando se prepare distribución. No bloquear desarrollo local por carecer hoy de credenciales, pero no publicar como firmado.
- [ ] Investigar el aviso del chunk `ChangesPane` (~1,08 MB JS minificado, ~331 kB gzip en la auditoría) y medir carga real; dividirlo si perjudica arranque o interacción. El aviso por sí solo no equivale a un fallo funcional.
- [ ] **Q9:** decidir si validar Linux con Tauri/PTY antes de D2 o seguir solo macOS, documentando el alcance. Cerrar la matriz D1 y SEC y actualizar la auditoría con evidencia nueva.

**Puerta P0 → P1:** D1 y seguridad conformes a v0.4, terminal nativa comprobada, tipos de sesión resueltos y discrepancias/decisiones documentadas. No abrir la implementación de agentes por una marca de «build verde».

## 3. P1 — Entrega 2: Agent Loop

El alcance de D2 no está implementado. No usar salida de PTY como fuente de verdad del agente ni asumir una API de OpenCode sin probarla.

### P1.1 Spike 2 y decisiones previas

- [ ] Probar la interfaz actual de OpenCode (servidor HTTP, SDK o proceso I/O) con la documentación de la versión elegida: iniciar/conectar sesión, eventos, estado, archivo activo, herramientas, mensaje, cancelación, permisos y reconexión. Registrar matriz observado/esperado, versión y capacidades reales (**Q2**). Si una señal no existe, limitar heurísticas PTY al estado y etiquetar la funcionalidad como best-effort, como dispone v0.4.
- [ ] Definir **Q3** (drafts/comentarios en datos Marvis, por checkout; qué hacer al borrar worktree), **Q4** (envío durante un turno: avisar y elegir enviar/encolar/cancelar) y **Q10** (cancelación no garantiza revertir cambios parciales; dejarlos visibles). Confirmar políticas de permisos/reintento y registrar casos no soportados.

**Puerta:** no construir la UX dependiente de eventos/permiso/cancelación antes de conocer las capacidades de la integración escogida.

### P1.2 Backend, estados y UX del agente

- [ ] Implementar `AgentBridge` y adaptador OpenCode; `AgentService` como dueño del proceso/conexión, con eventos normalizados, IDs validados, salida limpia y reconexión. Activar `SessionType agent` solo en checkout Git; contemplar OpenCode ausente, proceso muerto o desconexión sin pérdida de drafts.
- [ ] Flujo de una acción **nuevo worktree → abrir checkout → iniciar OpenCode** y también inicio desde un checkout Git existente. Sidebar/estado muestran actividad aun con checkout no enfocado: azul trabajando, ámbar necesita atención, rojo error; dos agentes simultáneos no pisan su estado. Barra global refleja pendientes.
- [ ] `Follow Agent`: seguir archivo activo; una selección manual pausa y ofrece reanudar; OFF por defecto durante review. Mensajes, solicitud y respuesta a permisos, cancelación y fallo/reinicio con capacidades reales del bridge.

**Evidencia:** pruebas del bridge con eventos/desconexión y ensayos reales D2-01…D2-03, D2-06…D2-08, D2-11…D2-14.

### P1.3 Review y cierre de loop

- [ ] Comentarios de línea/rango sobre diff, drafts editables/eliminables en SQLite por checkout, target de sesión cuando haya varios agentes y rondas de review. Persistir archivo/lado/línea/rango/hash; si cambia el destino, `outdated` y nunca reanclarlo a código ajeno por conjetura (reanchoring automático es D3).
- [ ] Enviar todos los drafts de una ronda como **un solo mensaje**; políticas de turno activo Q4 y recuperación tras reconexión sin duplicar envíos. Estados draft → sent → resolved. Tras un nuevo turno comparar líneas cambiadas y permitir marcar resuelto; mantener viewed progress.

**Avance 2026-09-26 (cuarta parte, sin cerrar la tarea):** cerrado el punto 3. Como 2.0.18 **no emite evento de fin de turno**, un turno terminado se **observa**: el composable de agente sondea cada 2 s **solo mientras hay un turno que él mismo vio arrancar**, y esa transición a ocioso es la única señal de fin. `busy` es **derivado, no reportado**: una sesión que nunca corrió tampoco tiene tiempo de inactividad, así que el backend expone `idle_at` (el hecho) y el cliente lo combina con los arranques que observó; sin esa distinción toda sesión nueva parecería ocupada y nada se cerraría nunca. Al contarse un turno, la ronda se cierra (`acked`), que significa que el turno terminó, no que las notas estén resueltas. El **veredicto por nota** lo da el diff, el único que sabe qué líneas existen: `unchanged` y `missing` permiten marcar resuelta, `changed` no (nada dice que el agente atendiera esa nota) y `unknown` cuando un diff grande todavía no cargó la línea, que no se puede afirmar como eliminada. Una nota `sent` cuya línea desapareció **no tiene fila** donde colgarse, así que se lista aparte: sin eso, el caso en el que más probablemente el agente actuó sería justo el que el usuario no podría ver. El estampado de `outdated` tras el turno no necesitó código nuevo: el diff se recarga con el cambio de estado Git y la verificación de anclas ya reacciona. Verificado contra un servidor real (`a_turn_is_observable_through_the_idle_time`). Sigue pendiente **Q4** (aviso de turno activo con enviar/encolar; la cancelación no es posible en esta versión) y el **loop con dos agentes**.

**Avance 2026-09-26 (quinta parte, sin cerrar la tarea):** ejecutado el **Bloque 4** — runbook de D2-04, D2-05, D2-09, D2-10, D2-13 y D2-15, con **dos agentes en dos checkouts distintos** (dos `opencode serve` simultáneos). Todo quedó registrado en **`docs/D2_RUNBOOK.md`** con pasos, evidencia y veredicto por caso. Resultado: **cinco pasaron, ninguno falló, uno no se ejecutó**. El loop real se cubre con el test gateado nuevo `two_checkouts_run_the_review_loop`: un solo mensaje con el marker y dos archivos, turno que termina, el agente editando ambos archivos, hash de ancla que ya no coincide, y el segundo checkout sin enterarse. D2-04 se cubre con `review_drafts_across_two_files_survive_a_database_restart`. **D2-13 queda NO EJECUTADO**: su paso definitorio es reiniciar la app con dos agentes vivos y comprobar el arranque, y eso no se corrió; los pasos de repos, layout y drafts sí pasaron por reinicio de la base. Hallazgo de la corrida: con un review **ambiguo** el modelo se queda deliberando y `time.idle` nunca llega aunque el archivo ya haya cambiado — Marvis lo vería como «trabajando» para siempre y **no hay cancelación posible**, así que la instrucción del review tiene que ser inequívoca. Sigue pendiente **Q4**.

- [ ] Validar dos agentes en checkouts distintos y el loop completo worktree → agente → diff → comentar → enviar → agente continúa → volver a revisar.

**Puerta P1 → P2:** D2-01…D2-15 documentados y reproducibles (incluidos permisos/cancelación y la limitación best-effort si la interfaz impide garantías), drafts persistentes tras reinicio, ninguna dependencia de scraping del PTY para la verdad del agente. Actualizar seguridad, CI y empaquetado antes de cerrar D2.

## 4. P2 — Entrega 3: Depth

Partir de casos reales de D2; no adelantar el reanclaje sin diffs de agentes. Mantener la aplicación plenamente usable aunque la evaluación de libghostty concluya que no conviene adoptarlo.

1. [ ] **Reanchoring / Q5:** guardar contexto y hash; encontrar una nueva posición solo con alta confianza, mostrar origen y destino; baja confianza → `outdated`, nunca asociación con código ajeno (**D3-01**).
2. [ ] **Previews ricos:** PNG/JPEG/WebP/GIF con dimensiones, fit/100 %/zoom y apertura externa, sin bloquear UI con imágenes grandes; SVG sin ejecutar scripts; JSON plegable y YAML formateado (**D3-02…D3-04**). No confundir imágenes relativas ya soportadas en Markdown con este visor completo.
3. [ ] **Neovim RPC:** descubrir socket por checkout, reutilizar instancia existente y saltar a línea exacta; socket ausente/obsoleto → inicio nuevo con mensaje (**D3-05…D3-06**). Integración bidireccional/plugin siguen opcionales.
4. [ ] **Renderer:** spike libghostty frente a WebView con los mismos datos de caudal, latencia, CPU, memoria, frame time y resize; decisión y reporte publicados (**D3-07**). Considerar aquí resultados de la tarea de ligaduras P0.2 sin posponer hasta D3 el requisito explícito del usuario.
5. [ ] **Diff:** modo lado a lado y stage/unstage por hunk con salvaguardas para archivos cambiantes, binarios y cambios concurrentes. Pruebas Git de ida/vuelta.
6. [ ] **Ergonomía:** wide/medium/narrow (inspector overlay y drawers en ancho estrecho), navegación completa por teclado, temas claro/oscuro del sistema y tamaño configurable de terminal/inspector (**D3-08…D3-09**).
7. [ ] **Rendimiento y distribución:** suite reproducible de benchmarks con resultados guardados y detección de regresiones (**D3-10**); memoria acotada al crecer sesiones, onboarding, errores, firmado/notarización y ruta de actualización para distribución.

**Puerta final:** D3-01…D3-10 con evidencias automatizadas y nativas; conservar el reporte de benchmarks, las decisiones Q1…Q12 aplicables y el estado de firma/publicación. Una tarea no probada permanece abierta.

## 5. Trazabilidad rápida de lo pendiente

| Grupo                   | Pendiente comprobable                                                                          | Referencia                              |
| ----------------------- | ---------------------------------------------------------------------------------------------- | --------------------------------------- |
| Riesgo inicial          | WebView/PTY y uso real; completar spike 0.2                                                    | D1-07/08/18, `terminal-spike/README.md` |
| D1 parcial              | Worktree→shell, watcher, diff grande, editores, Missing, actividad, reinicio y rama sin remoto | D1-05/09/11/13/14/16/17/19              |
| D1 no verificado        | `yes` 30 s e idle CPU con cinco sesiones                                                       | D1-08/18                                |
| D1 sin función completa | `server`/`custom`; ligaduras reales solicitadas                                                | v0.4 §5.3, requisito de usuario         |
| Seguridad               | Contrato de `..`, argumento shell aplicable y matriz de pertenencia                            | SEC-01/03/06                            |
| Lanzamiento             | WebKit/GUI, métricas, firma y notarización                                                     | v0.4 §5.6, §9, §10                      |
| Fases posteriores       | OpenCode, agentes y review; después Depth                                                      | D2-01…15, D3-01…10                      |

**Fuentes de verdad:** `MARVIS_DELIVERY_PLAN_v0.4.md` para alcance; `MVP_VALIDATION.md` para evidencia y estado de D1; este archivo para secuencia y trabajo pendiente. El antiguo `MVP_plan.md` es histórico. El traslado de documentos existentes no cambia el estado del código ni convierte una prueba pendiente en pasada.

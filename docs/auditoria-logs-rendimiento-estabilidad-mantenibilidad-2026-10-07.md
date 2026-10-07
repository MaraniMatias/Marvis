# Auditoría de logs, rendimiento, estabilidad y mantenibilidad

Fecha: **2026-10-07**. Aplicación: **Marvis 0.17.1**. Esquema: **14**.
Referencia: `HEAD 806c9ce` **más los cambios locales sin commit**, incluidos los nuevos composables de presentación.

## 1. Conclusión ejecutiva

Los cambios anteriores mejoran sustancialmente la aplicación: menos trabajo de fondo, lecturas compartidas, control de flujo del terminal, limpieza de listeners, límites para diffs y una política explícita de logs. **No permiten concluir que todo esté resuelto.**

La revisión identifica tres clases distintas de pendientes:

1. **Defectos y huecos demostrables en el código:** errores Git confundidos con ausencia, rutas de lectura sin plazo, memoria sin presupuesto en determinadas colas y pipes, limpieza incompleta en salidas excepcionales y respuestas de polling obsoletas.
2. **Un fallo observado en ejecución:** una DB de esquema 13 es rechazada correctamente, pero el arranque termina en `SIGABRT` y el motivo no queda en el log recomendado al usuario.
3. **Costes y riesgos que requieren medición:** normalización en scroll, serialización del input, geometría de scrollbars, recursos de Shiki y contención SQLite. No hay un perfil que permita afirmar cuánto afectan al usuario.

La evaluación de los agentes **no se aceptó por autoridad**. Una revisión independiente con GPT-6 Luna `xhigh` descartó cuatro candidatos, corrigió recomendaciones y encontró rutas omitidas. Este documento refleja los veredictos contrastados, no la suma acrítica de los informes.

Prioridades recomendadas:

| Orden | Tema                                                                          | Motivo                                                                                                                                     |
| ----- | ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| 1     | Arranque y diagnóstico — LS-01                                                | Fallo real; la app no llega a una sesión funcional.                                                                                        |
| 2     | Identidad y resultado de limpieza de procesos — LS-04, RV-06                  | No declarar muerto un proceso no verificable ni señalar un grupo reutilizado. Impacto potencial alto; escenarios adversos no reproducidos. |
| 3     | Semántica de errores y cobertura de plazos Git — RV-01 a RV-03                | Hay lecturas todavía ilimitadas y fallos que se presentan como datos válidos ausentes.                                                     |
| 4     | Presupuestos de recursos y salidas excepcionales — PM-11, RV-04, RV-05, RV-07 | Un límite de tiempo o de wakeups no es un límite de memoria ni una garantía de cleanup.                                                    |
| 5     | Polling y recursos Markdown — LS-07, PM-06                                    | Reintentos permanentes y una vista que puede conservar imágenes obsoletas.                                                                 |

No se propone migrar bases, recuperar formatos históricos ni subir la versión. En `v0.*.*`, rechazar un esquema incompatible es la política del proyecto; **el rechazo y la forma de comunicarlo son asuntos diferentes**.

## 2. Alcance, método y grado de evidencia

### 2.1 Trabajo realizado

- Dos auditorías secuenciales con MiniMax M3.1 Flash Preview: logs/estabilidad y rendimiento/mantenibilidad.
- Una revisión independiente con GPT-6 Luna `xhigh`, seguida de una segunda pasada del mismo revisor sobre Git, workspace, worktrees, review y configuración.
- Contraste final de rutas críticas por el agente principal antes de redactar este documento.
- Lectura de código, callers y consumidores; contraste con bibliotecas instaladas y artefactos del arranque anterior. No se modificó código durante esta evaluación.
- Los documentos preexistentes `docs/README.md`, `docs/marvis.md` y los cambios ajenos de `README.md` se preservaron.

Las referencias `ruta:línea` corresponden al árbol local auditado. Pueden desplazarse al aplicar cambios posteriores.

### 2.2 Áreas revisadas

| Área                       | Rutas principales                                                                                                                                                                                         |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Arranque, logs y salida    | `src-tauri/src/main.rs`, `src/main.ts`, `src/App.vue`, `src/presentation/toasts.ts`, `src-tauri/src/commands/app.rs`                                                                                      |
| Terminal                   | `src/components/TerminalSession.vue`, `src/lib/terminal-renderer.ts`, `src/lib/marvis-terminal.ts`, `src-tauri/src/terminal/mod.rs`, `src-tauri/src/commands/terminal.rs`                                 |
| Git y watchers             | `src-tauri/src/services/git.rs`, `src-tauri/src/git/mod.rs`, `src/presentation/git-watchers.ts`, `src/presentation/active-git-snapshot.ts`                                                                |
| Workspace y worktrees      | `src-tauri/src/services/workspace.rs`, `src-tauri/src/services/worktree.rs`; lectura dirigida de apertura, reconciliación, creación, preflight y eliminación                                              |
| Agentes y review           | `src-tauri/src/services/agent.rs`, `src-tauri/src/services/opencode.rs`, `src/presentation/agent-sessions.ts`, `src-tauri/src/services/review_round.rs`, transiciones de review en persistencia           |
| Documentos, diff y edición | `src/components/DocumentPane.vue`, `src/components/FileDiff.vue`, `src/components/MainPane.vue`, `src/lib/code-editor.ts`, `src/lib/front-matter.ts`, `src/lib/markdown-preview.ts`                       |
| Coordinación y cachés      | `src/presentation/diff-loader.ts`, `source-highlight.ts`, `changed-lines.ts`, `layout-persistence.ts`, `markdown-preview.ts`, `use-large-diff.ts`; `src/lib/source-highlighter.ts`, `diff-highlighter.ts` |
| DB, archivos y settings    | `src-tauri/src/persistence/mod.rs`, `src-tauri/src/services/files.rs`, `src-tauri/src/config.rs`, `src/domain/ui-state.ts`, `src/lib/ipc.ts`                                                              |
| Render y artefactos        | `src/components/OverlayScrollbar.vue`, rutas virtualizadas de inspector/diff, `vite.config.ts`, `scripts/xterm-debug-log-plugin.mjs`, scripts de build y tests asociados                                  |

Esto **no equivale a verificar cada línea del repositorio**. La revisión de servicios extensos fue dirigida por flujos y riesgos. Tampoco equivale a una auditoría integral de seguridad, accesibilidad o dependencias externas.

### 2.3 Etiquetas utilizadas

- **Código:** el comportamiento está demostrado por una ruta alcanzable del árbol actual; no se ejecutó necesariamente el escenario.
- **Runtime previo:** existen artefactos de una ejecución anterior observada. No se repitió en esta fase.
- **Riesgo:** la condición peligrosa o el coste existe, pero falta reproducir su consecuencia o medir su importancia.
- **Descartado:** la premisa no coincide con el flujo real; no justifica cambios.
- **Decisión deliberada:** existe una limitación aceptada, no un defecto por sí misma.

“Confirmado en código” **no significa** que se haya reproducido una fuga, pérdida de datos, bloqueo visible o caída por falta de memoria. La prioridad combina impacto, alcance y evidencia; no mide frecuencia observada.

## 3. Relación con la auditoría y los cambios anteriores

El historial disponible conserva el resumen de la auditoría inicial, pero no su listado literal completo de 29 puntos. Por ello se relacionan **principios y áreas**, sin inventar una correspondencia numérica exacta.

| Principio inicial                    | Mejora realmente presente                                                                                               | Lo que todavía no queda cerrado                                                                                                      |
| ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Reducir ruido sin ocultar errores    | Nivel `Debug`, filtros `ureq`/`ureq_proto`, rotación acotada, sin stdout en release; avisos SSE limitados y sin payload | Startup sin diagnóstico durable, consola/frontend sin ruta durable, contexto de xterm eliminado en build, errores Git inconsistentes |
| Evitar trabajo duplicado             | Single-flight Git/candidatos, coalescing del polling, deduplicación de filas y refresh del documento                    | Ownership del highlighter en vuelos simultáneos, input IPC por evento y consultas DB repetidas; costes por medir                     |
| Acotar recursos                      | Diffs/páginas/blob con límites, watcher con wakeup coalescido, presupuesto nominal del source cache, marcas de agua PTY | Payload del watcher, pipes genéricos y fail-open no constituyen cotas duras de memoria                                               |
| Evitar carreras y respuestas tardías | Generaciones y guards en composables, liberación de listeners registrados tarde, lifecycle de stores                    | `catch` del polling de terminal; `finally` del highlighter; recursos Markdown no invalidados                                         |
| Evitar cuelgues Git                  | Runner con plazo y process group; ocho lecturas de `git/mod.rs` lo reutilizan                                           | Lecturas en `worktree.rs`/`workspace.rs` todavía usan `.output()`/`.status()` sin plazo                                              |
| Hacer fiables guardado y cierre      | Exclusión por path, revalidación de contenido, rename y sync de directorio; deadline de cierre                          | CAS externo no garantizado; diagnóstico de cierre efímero; salidas excepcionales de procesos incompletas                             |
| Mantener código limpio               | Cuatro coordinadores concretos extraídos; versionado por blob y defaults heredados retirados                            | No confundir validación/config humana/API externa con legacy; documentar semánticas y límites verdaderos                             |

Correcciones explícitas a conclusiones anteriores:

- `git/mod.rs` **no contiene solo escrituras**: las ocho operaciones vivas inventariadas allí son lecturas.
- Acotar stderr **no redacta rutas**.
- No se encontró la doble disposición de xterm en las rutas inspeccionadas; **no está demostrado que sea imposible ni que la dependencia sea la causa**.
- Tests verdes y build frontend correcto **no prueban** que la app empaquetada arranque o que el rendimiento haya mejorado.
- Quitar compatibilidad histórica no implica rechazar todo campo JSON desconocido. El decoder sigue aceptando campos extra; eso no demuestra una migración ni una rama legacy.

## 4. Hallazgos: logs y estabilidad

### LS-01 — Rechazo de DB convertido en abort, sin diagnóstico durable

**Prioridad alta. Evidencia: código + runtime previo.**

- **Ubicación:** `src-tauri/src/persistence/mod.rs:2334-2347`; `src-tauri/src/main.rs:150-165`.
- **Ruta:** `Database::open` rechaza esquema 13 frente a 14; `setup` propaga el error con `?`. En la ejecución observada, Tauri 2.12 panica al fallar el setup y el callback nativo de `tao` no puede desenrollar: termina en `SIGABRT`.
- **Evidencia operativa:** `/tmp/marvis-app.log` contiene el rechazo y el stack de abort. El `Marvis.log` consultado no contiene ese motivo. El proceso no llegó a una app funcional para probar con MCP.
- **Impacto:** la app no abre; quien la lanza gráficamente no recibe una explicación útil ni la encuentra en el archivo recomendado para soporte. No se afirma que este sea el único error de startup capaz de abortar.
- **Recomendación:** registrar una vez el fallo de apertura/inicialización y separar el rechazo esperado del camino de panic. Presentar un mensaje accionable y salir de forma controlada, sin migrar ni borrar automáticamente la base. **Añadir solo `log::error!` resuelve diagnóstico, no el abort.**
- **Aceptación:** fixture incompatible; DB intacta; log con causa/esquemas; mensaje legible y salida controlada. Repetir en debug y en binario empaquetado lanzado sin terminal.

**LS-02, consecuencia documental:** `README.md:114-121` promete que el log nombra el rechazo. Los artefactos lo contradicen. Alinear documentación y conducta verificada; no contabilizarlo como una segunda caída independiente.

### LS-03 — Diagnósticos Git sin presupuesto uniforme ni redacción real

**Prioridad media. Evidencia: código.**

- **Ubicación:** `src-tauri/src/services/git.rs:3243-3250`, frente a `:3085-3105`; `src-tauri/src/git/mod.rs:225-233` también incorpora stderr completo.
- **Mecanismo:** `git_error` devuelve stderr íntegro; el timeout usa un resumen de tres líneas y 120 caracteres por línea. El resumen retiene texto, incluidas posibles rutas: no es un filtro de privacidad.
- **Escenario:** una consulta falla con mensajes largos, paths absolutos o varias advertencias; el IPC puede entregar ese texto a la superficie de error de la aplicación.
- **Impacto:** diagnóstico voluminoso/inconsistente y posible exposición del layout local en capturas o reportes compartidos. Ver los propios paths dentro de la app no constituye por sí solo una vulnerabilidad.
- **Recomendación:** un presupuesto uniforme de mensajes, conservando código y causa. Si la política excluye rutas, aplicar redacción explícita; no prometerla por truncado. No adjuntar cuerpos de diffs o documentos como fallback de diagnóstico.
- **Aceptación:** stderr extenso, Unicode, vacío y con paths; comprobar tamaño, causa útil y redacción si se adopta. Cubrir también los consumidores fuera de `services/git.rs`.

### LS-04 — `EPERM` se interpreta como grupo terminado sin demostrarlo

**Prioridad alta por impacto potencial; frecuencia desconocida. Evidencia: clasificación en código, consecuencia no reproducida.**

- **Ubicación:** `src-tauri/src/terminal/mod.rs:604-612`, `:628-704`; pruebas del caso zombie en `:1399-1463`.
- **Mecanismo:** `signal_group_with` convierte tanto `ESRCH` como `EPERM` en `Gone`. El comentario justifica `EPERM` mediante zombies de macOS, pero el código se aplica a Unix y pertenecer a una sesión no demuestra que todos sus procesos conserven las mismas credenciales.
- **Escenario adverso:** un job privilegiado deja un miembro vivo no señalable en el foreground group. El kernel puede rechazar la señal con `EPERM`; el caller recibe éxito porque solo comprueba liveness cuando el resultado es `Unverified`.
- **Impacto:** cierre declarado exitoso sin demostrar que acabaron los miembros del grupo. No se reprodujo con `sudo` ni se ejecutó esta revisión en Linux.
- **Recomendación:** mantener distintos “no existe”, “no pude verificar/señalar” y “señal enviada”. Limitar cualquier excepción de zombies a evidencia de plataforma suficiente; conservar el error/estado de cleanup para reintentar en los casos ambiguos.
- **Aceptación:** tests de error inyectado con grupo todavía existente y pruebas macOS/Linux de zombies y permisos. No reemplazar el problema por una señal indiscriminada a grupos no verificados.

La propuesta inicial de consultar `process_group_exists` con la condición invertida fue rechazada. Un warning tampoco demuestra que el grupo esté muerto. La revisión no afirma que el test actual haya fallado en CI Linux: falta esa ejecución.

### LS-05 — Falta una ruta durable y acotada para errores del frontend

**Prioridad media. Evidencia: código.**

- **Ubicación:** `src/main.ts:1-13`; `src/presentation/toasts.ts:14,79-84`; `src/presentation/git-watchers.ts:145-150`; targets en `src-tauri/src/main.rs:65-78`.
- **Mecanismo:** los errores renderizados quedan en estado local/toasts; no se encontró captura global de errores/rechazos con destino durable. El error de consola del watcher sí tiene una indicación persistente de “unwatched” en la UI, pero la consola no aporta por sí misma un archivo para soporte.
- **Impacto:** incidentes frontend no presentes en el log Rust; especialmente relevante para errores inesperados o avisos que desaparecen con la ventana. No se propone registrar cada fallo ya explicado satisfactoriamente en pantalla.
- **Recomendación:** ruta explícita frontend → logger nativo, con categorías/códigos, tamaño máximo, deduplicación y rate limit. Elegir qué errores globales registrar y excluir contenido de terminal, teclas, documentos y payloads del agente.
- **Aceptación:** provocar un error controlado en una app empaquetada; encontrar un registro útil y acotado; verificar que no incluye contenido sensible ni genera bucles de logging.

**Corrección del informe original:** `TargetKind::Webview` transmite logs nativos al webview; añadirlo y reenviarlos a `console` **no crea frontend → archivo**.

### LS-06 — Avisos de cierre sin persistencia y trabajo que sobrevive al presupuesto

**Prioridad baja/media. Evidencia: código; pérdida efectiva no reproducida.**

- **Ubicación:** `src/App.vue:1055-1070`, `:1194-1233`.
- **Mecanismo:** `withinDeadline` informa mediante toast y resuelve al agotar 5000 ms; después continúa el sweep/cierre. La operación original no se cancela. Puede terminar o fallar más tarde sin que cambie la promesa ya resuelta.
- **Escenario:** guardado de UI/settings pendiente sobre disco lento; vence el presupuesto y se cierra la ventana. No hay copia durable del aviso.
- **Impacto:** explicación insuficiente de un estado que podría quedar sin confirmar. No se afirma que el toast desaparezca inmediatamente: el sweep puede consumir otro presupuesto.
- **Recomendación:** mantener el cierre acotado, registrar de forma durable qué operación quedó incierta y distinguir timeout de fallo confirmado. No guardar un marcador en preferencias solo para este diagnóstico ni introducir un diálogo bloqueante automáticamente.
- **Aceptación:** promesa que no termina y promesa que rechaza después del timeout; cierre dentro del presupuesto, diagnóstico durable y sin rechazo no manejado. Comprobar el estado restaurado en el siguiente launch.

### LS-07 — Polling de terminal sin guard equivalente en errores ni política de recuperación

**Prioridad media. Evidencia: código.**

- **Ubicación:** `src/components/TerminalSession.vue:333-377`; tests en `src/components/TerminalSession.test.ts:803-814,1123-1206`.
- **Mecanismo:** el éxito verifica `disposed` y session ID. El `catch` llama `showError` sin esas comprobaciones. Mientras el estado continúe `running`, un rechazo persistente no detiene ni ralentiza el intervalo; no hay guard de lectura en vuelo en esta función.
- **Escenario:** sesión borrada o ownership cambiado; respuesta fallida que llega después de mover/desmontar el panel; backend más lento que el intervalo.
- **Carga derivable:** ritmo visible de 750 ms ≈ 1,33 llamadas/s por panel; oculto de 5000 ms = 12/min. Una misma cadena de error **no implica** un rerender por tick.
- **Recomendación:** verificar lifecycle, sesión y checkout también al rechazar; distinguir fallos terminales de transitorios; coalescer lecturas y aplicar backoff donde corresponda. Reinvocar `syncStatusPolling` sin cambiar estado/política no basta para detener el timer.
- **Aceptación:** fake timers con error persistente, error recuperable, respuesta obsoleta y lectura lenta; no solapamiento ilimitado ni escrituras tardías; recuperación y detección de salida conservadas.

### LS-08 — Los errores de worktree consultan stdout y pueden perder la causa

**Prioridad media. Evidencia: código contrastado por el agente principal.**

- **Ubicación:** `src-tauri/src/services/worktree.rs:918-940`.
- **Mecanismo:** `checked_git` detecta exit no exitoso y llama a `git_error`; este formatea `output_text`, que lee solo stdout. La causa emitida únicamente por stderr desaparece.
- **Escenario:** Git responde con stdout vacío y `fatal: ...` en stderr. La UI recibe la etiqueta de la acción, pero no el motivo.
- **Impacto:** soporte y recuperación deficientes en creación/eliminación/preflight de worktrees; contraste directo con otros runners que sí conservan stderr.
- **Recomendación:** diagnóstico compartido y acotado que prefiera stderr, sin modificar la política de ejecución de escrituras.
- **Aceptación:** un `Output` no exitoso con stdout vacío y stderr útil conserva causa y código; mensajes largos siguen el presupuesto de LS-03.

## 5. Hallazgos: rendimiento y coordinación frontend

### PM-01 — Normalización completa de UI state en una ruta de scroll

**Prioridad media; medir antes de calificar impacto alto. Evidencia: código.**

- **Ubicación:** `src/App.vue:835-854,1762-1773`; `src/domain/ui-state.ts:127-164`.
- **Mecanismo:** un parche de posición reconstruye y normaliza el estado completo, incluyendo validación de paths y deduplicación de hasta 2000 directorios expandidos. También se reconstruyen objetos reactivos/props.
- **Impacto posible:** asignaciones y trabajo proporcional a directorios expandidos en una interacción frecuente. La frecuencia real de eventos y el coste de render no fueron medidos.
- **Recomendación:** especializar el parche interno de scroll sobre estado previamente validado, manteniendo la validación completa en límites IPC/persistencia; consolidar eventos solo si el perfil lo justifica.
- **Aceptación:** misma persistencia/validación; un parche de scroll no reprocesa directorios inalterados. Perfil WKWebView con 0, 300 y 2000 directorios; comparar CPU, asignaciones y latencia de scroll.

No se mantienen las estimaciones “90 % menos trabajo” ni “18–36 mil splits/s” como resultados: eran escenarios hipotéticos, no mediciones.

### PM-02 — Coste de serialización e IPC por evento de entrada del terminal

**Prioridad media, coste percibido no medido. Evidencia: código.**

- **Ubicación:** `src/components/TerminalSession.vue:443-453,628`; `src/lib/ipc.ts:305-307`; `src-tauri/src/commands/terminal.rs:236-250,344-364`.
- **Mecanismo:** input FIFO por evento; `Array.from(bytes)` convierte el buffer en números para invoke JSON. El backend deserializa `Vec<u8>` y valida ownership en SQLite antes de escribir.
- **Impacto posible:** payload/copia ampliados en pegados grandes y una validación por evento. El número de eventos de un paste no está fijado; no se presupone un único payload de 1 MB.
- **Recomendación:** medir payloads y latencia interactiva. Verificar primero si el transporte binario nativo de la versión instalada satisface metadata y permisos; si se coalesce, conservar FIFO y evitar añadir latencia visible a cada tecla.
- **Aceptación:** Unicode y secuencias de control preservados byte a byte; pegados grandes, entrada durante close/move y ownership inválido; comparar throughput y latencia de tecla antes/después.

RAF/base64 no se acepta como arreglo automático: puede añadir espera y nuevas copias. No retirar la validación de ownership para ahorrar una consulta.

### PM-05 — Medidas de geometría por evento en `OverlayScrollbar`

**Prioridad baja/media. Evidencia: código; forced reflow no demostrado.**

- **Ubicación:** `src/components/OverlayScrollbar.vue:82-117,139-142`.
- **Mecanismo:** el handler lee rects y métricas de scroll antes del early-out que evita escrituras de estilos iguales.
- **Impacto posible:** trabajo repetido en todos los paneles con scrollbar. Un layout limpio puede responder barato; las lecturas no prueban un reflow forzado.
- **Recomendación:** perfil antes de añadir caching; consolidar por frame si aporta. Una caché basada solo en `ResizeObserver` puede quedar incorrecta cuando se mueve un ancestro sin cambiar tamaño.
- **Aceptación:** perfil con layout invalidado y limpio; thumb y coordenadas correctos al redimensionar, mover paneles y cambiar zoom. Contar actualizaciones por frame sin convertir ese conteo en una mejora de FPS inventada.

### PM-06 — Preview Markdown conserva imágenes viejas si el texto no cambia

**Prioridad media. Evidencia: ruta de código; no reproducción runtime.**

- **Ubicación:** `src-tauri/src/services/git.rs:675-679` → `src/App.vue:1007-1015` → `src/components/DocumentPane.vue:517-519,551-553,658-662` → `src/presentation/markdown-preview.ts:69-88`.
- **Mecanismo:** la actividad incrementa una revisión del checkout. El documento se relee, pero si el Markdown es idéntico se retorna antes de resolver de nuevo sus imágenes. El HTML mantiene el data URI anterior.
- **Escenario:** preview abierto con `![imagen](figura.png)`; otro proceso sustituye `figura.png` sin tocar el `.md`. La imagen puede permanecer vieja hasta una nueva carga efectiva.
- **Impacto:** vista obsoleta, resultado de una optimización válida para texto aplicada también a recursos dependientes.
- **Recomendación:** separar identidad del Markdown e invalidación de sus recursos. Conservar el HTML/texto y refrescar dependencias afectadas; no renderizar ni releer todas las imágenes ante cada evento del checkout.
- **Aceptación:** modificar, borrar y recrear una imagen referenciada con Markdown idéntico; actualizar la imagen o advertencia, mantener posición y no disparar nuevos renders por actividad ajena.

**Coste secundario por medir:** `src/lib/markdown-preview.ts:87-118,180-187,260-278` crea un parser por render y realiza dos ciclos explícitos de parse/serialización DOM. Reutilizarlo exige aislar los callbacks/estado por render. **No hay un render por cada actividad**: el early return precisamente evita esa ruta. Debounce de 220 ms y batch máximo de 1 s tampoco significan un máximo global de 1 Hz.

### PM-07 — Trabajo O(N) por edición Markdown para detectar front matter

**Prioridad media. Evidencia: código.**

- **Ubicación:** `src/lib/code-editor.ts:362-364,391-410,471-472`; `src/lib/front-matter.ts:33-37`.
- **Mecanismo:** el listener materializa el documento completo; el `StateField` Markdown lo materializa otra vez al editar. La detección divide el texto entero aunque solo interesen las líneas iniciales y no exista front matter.
- **Impacto posible:** coste proporcional al tamaño del documento por tecla. Solo se instala para Markdown; no se extiende la conclusión a todos los lenguajes.
- **Recomendación:** localizar y leer el rango inicial directamente desde `Text`; serializar solo donde el draft realmente necesita la cadena completa. No dejar de actualizar `content`/draft/dirty state sin verificar sus consumidores.
- **Aceptación:** YAML válido, sin front matter, CRLF y fence sin cierre; equivalencia de decoraciones y guardado. Perfil de edición en documentos Markdown grandes, con y sin bloque inicial.

Asignar la cadena a una ref no demuestra una tercera copia del documento ni una cifra concreta de MB/s.

### PM-08 — El presupuesto de source cache no está expresado en bytes reales

**Prioridad media para exactitud del contrato; coste de eviction no medido. Evidencia: código.**

- **Ubicación:** `src/lib/source-highlighter.ts:547-635`.
- **Mecanismo:** `bytes` suma `source.length` y longitudes de markup. Son unidades UTF-16, no tamaño UTF-8 ni consumo total de heap. `oldest()` escanea el mapa por cada expulsión; una oleada puede requerir trabajo cuadrático en el número de entradas.
- **Impacto:** el nominal “4 MiB” no garantiza esa cota de memoria. Gramáticas, renders en vuelo, objetos y referencias externas quedan fuera del presupuesto.
- **Recomendación:** documentar/nombrar la unidad o adoptar una estimación conservadora; no vender `TextEncoder` como medición exacta de heap. Mejorar eviction solo si el perfil demuestra coste relevante.
- **Aceptación:** Latin-1, CJK, emoji, markup grande y eviction de promises pendientes; contabilidad coherente con la unidad elegida. Medir memoria de WKWebView/JavaScriptCore, no extrapolar representaciones V8.

El helper `set` no descuenta un reemplazo, pero los callers actuales hacen `get`/`set` sin await intermedio. No se presenta esa debilidad teórica como fuga alcanzable demostrada.

### PM-10 — El bookkeeping del highlighter no tiene ownership del vuelo

**Prioridad baja/media. Evidencia: código; colores incorrectos no demostrados.**

- **Ubicación:** `src/presentation/diff-loader.ts:183-184,208-238`.
- **Mecanismo:** `highlighterReading` guarda solo path; una ruta distinta puede iniciar en paralelo. El `finally` antiguo pone el indicador en `null` y consume la cola sin comprobar que todavía le pertenezcan.
- **Escenario:** A comienza; el usuario cambia a B; A termina mientras B sigue en vuelo. La marca global puede dejar de describir B y permitir trabajo duplicado. Mismo path en otro checkout se serializa por coincidencia de string.
- **Impacto:** coalescing frágil/trabajo extra. Los guards de generación/checkout/diff evitan publicar resultados obsoletos en las rutas examinadas.
- **Recomendación:** ownership/generación del vuelo y una clave que represente checkout/path. Cambiar solo el string a una clave compuesta no arregla el `finally` obsoleto.
- **Aceptación:** terminar A/B/C en órdenes distintos, cambiar de checkout con mismo path y desmontar con cola pendiente; publicar solo el resultado vigente y acotar lecturas redundantes.

### PM-11 — Watcher con wakeups acotados, pero payload acumulado sin límite

**Prioridad alta de robustez; agotamiento no reproducido. Evidencia: código.**

- **Ubicación:** `src-tauri/src/services/git.rs:230-248,280-307,792-877,899-948,2879-2902`.
- **Mecanismo:** `sync_channel(1)` limita señales, no `merged.paths`. Se acumulan paths únicos en sets; luego se clonan y se mandan a `check-ignore`. Mientras el worker espera a Git, nuevos eventos continúan fusionándose.
- **Escenario:** compilación/generador que crea muchísimas rutas, junto con una lectura Git lenta. El slot puede acumular más de una ventana nominal de un segundo.
- **Impacto:** consumo de memoria y coste de clonación/serialización proporcional a rutas únicas. La observación recursiva sigue viendo ignorados; el filtro posterior evita invalidaciones, no el coste inicial de recibirlos.
- **Recomendación:** presupuesto por cantidad/bytes. Al superar el presupuesto, convertir el checkout en “requiere refresh completo” conservando la señal de actividad; no descartar silenciosamente paths ni afirmar que no hubo cambios.
- **Aceptación:** worker detenido/lento con ráfaga masiva; payload acotado, refresh conservador tras overflow y actividad preservada. Medir RSS y procesos `check-ignore` en un build de muchas rutas.

### PM-12 — Un evento global de flujo se parsea por cada terminal

**Prioridad baja; coste real no medido. Evidencia: código.**

- **Ubicación:** `src-tauri/src/commands/terminal.rs:88-114`; `src/components/TerminalSession.vue:155-159`; `src/lib/terminal-renderer.ts:20,59-70`.
- **Mecanismo:** un listener por sesión deserializa el payload antes de comparar ID. Cada reporte cuesta O(N) callbacks; con N terminales ruidosos, el agregado puede aproximarse a O(N²).
- **Impacto posible:** coste de parse/dispatch; payload pequeño. Terminales inactivos no emiten continuamente por este mecanismo.
- **Recomendación:** medir antes de introducir un dispatcher/mapa global. No usar búsquedas de substring en JSON como filtro de identidad.
- **Aceptación:** 1/10/30 terminales, salida simultánea y cierre; contar callbacks, CPU y latencia. Refactor solo si aporta, manteniendo release de gates/listeners por sesión.

## 6. Hallazgos adicionales de la revisión independiente

### RV-01 — Todavía hay lecturas Git sin deadline fuera de los módulos endurecidos

**Prioridad alta. Evidencia: código.**

- **Ubicación:** `src-tauri/src/services/worktree.rs:905-916`, callers en `:306-338,600-605,664-699`; `src-tauri/src/services/workspace.rs:430-460,1094-1123`.
- **Mecanismo:** el runner genérico de worktree usa `.output()` para lecturas y escrituras. `git_common_dir` y la validación de default branch también esperan sin plazo.
- **Escenario:** Git bloquea al leer config/FIFO o una consulta externa no termina durante apertura, locate o preflight de eliminación. La operación de UI puede permanecer pendiente indefinidamente.
- **Impacto:** feedback que no termina y ocupación de workers. No se afirma que ejecute en el hilo principal ni que congele directamente toda la UI.
- **Recomendación:** inventariar subcomando/flags y reutilizar el runner de lectura con plazo, manteniendo separada la política de mutaciones. No aplicar timeout indiscriminado al runner compartido.
- **Aceptación:** lectura colgada en cada preflight/locate termina con error dentro del presupuesto, mata/recoge procesos de forma segura y no ejecuta la mutación posterior.

### RV-02 — `optional_ref` sigue confundiendo fallos de Git con ausencia legítima

**Prioridad media/alta según campo. Evidencia: código.**

- **Ubicación:** `src-tauri/src/git/mod.rs:115-127,257-266,367-403`.
- **Mecanismo:** se corrigieron spawn/timeout/pipe errors, pero todo exit status no exitoso todavía se convierte en `Absent`: error fatal, terminación por señal y ausencia legítima comparten resultado.
- **Escenario:** config/ref ilegible o corrupta durante una consulta opcional. El caller puede dejar el campo vacío sin el warning reservado para `Err`.
- **Impacto:** información engañosa y errores ocultos. La política de permitir abrir un repo válido sin default branch puede mantenerse; lo incorrecto es atribuir un fallo a “no existe”.
- **Recomendación:** clasificación específica por operación y respuestas esperadas. No asumir universalmente “exit 1 = ausencia”: un `HEAD` unborn puede usar otra salida en una consulta distinta.
- **Aceptación:** detached HEAD, unborn, sin remoto y sin default branch siguen admitidos; error fatal/config inaccesible/señal se diferencian y se comunican según la política del campo.

### RV-03 — Un repo que falla en `show-toplevel` puede registrarse como carpeta plain

**Prioridad alta de corrección. Evidencia: código; escenario no ejecutado en esta fase.**

- **Ubicación:** `src-tauri/src/git/mod.rs:206-208`; `src-tauri/src/services/workspace.rs:47-74,929-933,1047-1067`.
- **Mecanismo:** todo fallo de `rev-parse --show-toplevel` devuelve `Ok(None)`; apertura interpreta `None` como carpeta normal y reconciliación como ausencia de actualización.
- **Escenario:** una carpeta que sí es checkout Git, pero la consulta falla por config corrupta/permisos. La app puede clasificarla como plain en vez de devolver la causa.
- **Impacto:** registro incorrecto y ocultación del estado Git. Distinguirlo de una carpeta realmente no Git.
- **Recomendación:** reconocer solo la respuesta legítima de “no es repo”; propagar los demás errores sin alterar el registro. No usar `.git` superficialmente como única prueba: hay worktrees/submódulos.
- **Aceptación:** plain, unborn, worktree y submódulo válidos; configuración corrupta/inaccesible produce error y no una nueva entrada plain.

### RV-04 — Crear threads del runner puede fallar sin cleanup completo

**Prioridad media, dependiente de agotamiento de recursos. Evidencia: código.**

- **Ubicación:** `src-tauri/src/services/git.rs:2963-2980,3191-3203`.
- **Mecanismo:** si falla el thread escritor, `?` retorna después de iniciar Git sin kill/wait. Si falla un drainer, `.expect` panica sin cleanup explícito del hijo.
- **Matiz:** el único input caller actual es `check-ignore --stdin`; cerrar stdin probablemente permite EOF. No se demuestra un huérfano permanente, pero no hay supervisión/reap garantizados en esa salida.
- **Impacto:** proceso no recogido o superviviente, panic de worker y posible retención de pipes bajo presión de recursos.
- **Recomendación:** cleanup estructurado del hijo en toda salida temprana y propagación de fallos de creación de lectores; no convertirlos en éxito vacío.
- **Aceptación:** inyectar fallo del escritor y de cada lector; error útil, sin panic no controlado, hijo recogido y helpers tratados con identidad segura.

### RV-05 — Error de `try_wait` termina al líder, no al grupo

**Prioridad media; condición rara no reproducida. Evidencia: código.**

- **Ubicación:** `src-tauri/src/services/git.rs:2987-2999`, frente a `:3003-3006,3052-3063`.
- **Mecanismo:** el camino de error usa `child.kill()/wait()` y recoge diagnóstico con plazo. No aplica la limpieza de grupo usada al vencer el deadline; un helper puede seguir reteniendo pipes.
- **Impacto:** limpiar el proceso directo no asegura terminar el trabajo que inició. Un timeout del drenaje tampoco termina al helper.
- **Recomendación:** unificar garantías de cleanup mientras la identidad del grupo esté estable; tener en cuenta RV-06 antes de reutilizar ciegamente `stop_git_read`.
- **Aceptación:** error de espera inyectado con helper vivo; retorno acotado, sin helper superviviente ni señales a terceros.

### RV-06 — Pipe retenido no demuestra que el PGID original siga reservado

**Prioridad alta por impacto potencial; carrera no reproducida. Evidencia: código y contradicción de invariantes.**

- **Ubicación:** `src-tauri/src/services/git.rs:2917-2922,3011-3025,3052-3063`.
- **Mecanismo:** después de recoger al líder, `StillHeld` provoca señal a `-child.id()`. El comentario supone que el escritor pertenece al grupo original y mantiene su número reservado. Otro comentario admite que un helper puede salir de ese grupo conservando el pipe.
- **Escenario adverso:** helper hace `setsid`; líder y grupo original desaparecen; el número se reutiliza antes del SIGKILL. Tener un fd abierto no vincula al escritor con el PGID que se va a señalar.
- **Impacto potencial:** señal a un grupo ajeno del mismo usuario o limpieza que no alcanza al helper escapado. No se observó reutilización ni señal incorrecta en ejecución.
- **Recomendación:** no deducir ownership por el pipe. Preservar identidad estable hasta señalizar, verificar pertenencia con garantías suficientes o abstenerse de señalar tras reap cuando no pueda probarse. Documentar límites para helpers escapados.
- **Aceptación:** helper retenedor dentro del grupo y helper escapado; limpieza cuando sea segura, ausencia de señales a grupos ajenos. Un test happy path de hijo retenedor no demuestra seguridad contra reutilización de PGID.

### RV-07 — Los pipes genéricos no tienen límite de bytes

**Prioridad alta de robustez; caída por memoria no reproducida. Evidencia: código.**

- **Ubicación:** `src-tauri/src/services/git.rs:3191-3203`, especialmente `read_to_end(&mut Vec)` en `:3198`.
- **Mecanismo:** stdout y stderr crecen hasta EOF en threads separados. El deadline de proceso limita duración, no volumen. Los límites del scanner de diff/blob no se aplican a todos los callers del runner genérico.
- **Escenario:** `status` con muchísimos paths u otra salida masiva durante una lectura permitida; amplificación con consultas concurrentes.
- **Impacto:** memoria proporcional a salida completa antes de resumirla o parsearla. Reducir el mensaje final no reduce lo ya acumulado.
- **Recomendación:** presupuestos específicos por consulta/stream y respuesta explícita al excederlos; cleanup/reap seguro. No devolver `Ok` con salida truncada como si representara el estado completo del repo.
- **Aceptación:** exceder límites en stdout y stderr; memoria acotada, causa distinguible de “sin cambios”, proceso terminado/recogido y funcionamiento normal bajo límites.

## 7. Riesgos medibles y decisiones deliberadas, no bugs demostrados

### PM-03 — Conexión SQLite única

`src-tauri/src/persistence/mod.rs:139-142,195-223` comparte una conexión bajo mutex. Serializa DB y puede añadir espera, pero las consultas suelen ser cortas y Git trabaja fuera de ese lock. **No hay evidencia de cuello de botella.**

Medir tiempo de espera y ejecución por consulta antes de introducir pools, WAL o prepared-statement caches. WAL no paraleliza llamadas sobre la misma conexión. No se afirma que el driver carezca de su timeout por defecto simplemente porque la app no lo configure.

### PM-09 — Gramáticas retenidas durante la vida de la ventana

`src/lib/source-highlighter.ts:449-473` conserva promises de highlighters exitosos. El catálogo es fijo/allowlisted: crecimiento finito, no entrada arbitraria ilimitada. Hay riesgo de memoria en sesiones con muchos lenguajes, pero falta medirlo.

No introducir un LRU con `dispose()` sin gestionar consumidores y llamadas activas: source y diff comparten instancias. Medir primero en WKWebView y verificar lifetime de referencias.

### Control de flujo PTY: fail-open deliberado

El gate usa **2 MiB alto / 512 KiB bajo / 5 s de timeout**, `delivered - parsed` y estado `paused`. Cuenta antes de enviar; ACK acumulativo monotónico; release al fallar envío/desmontar. Esto corrige los defectos anteriores de histéresis y contabilidad.

No es una cota dura de memoria: al faltar ACKs, fail-open favorece continuar el output. Un renderer suspendido o callback que nunca termina puede permitir acumulación. Medir salida sostenida, suspensión/resume, ACK perdido y close; no presentar el mecanismo como garantía de “nunca más de 2 MiB”.

La sospecha de una pequeña deuda residual sin ACK **no se validó como defecto**. El paso de reporte de 256 KiB está por debajo de la marca baja; no se encontró un caso actual que quedara detenido por esa sola condición.

### Guardado: atomicidad local, no CAS entre procesos

`src-tauri/src/services/files.rs:462-478,480-575,648-702` serializa por path dentro de Marvis, comprueba contenido esperado y hace rename/sync. Otro proceso todavía puede modificar el archivo entre comparación y reemplazo. No prometer exclusión externa.

Un fallo de sync del directorio después del rename significa **contenido reemplazado con durabilidad incierta**, no “no se escribió”. El mensaje actual distingue ese resultado; cualquier cambio debe preservar esa honestidad. No se observó pérdida de datos en esta auditoría.

### Mutaciones Git sin timeout

Creación/eliminación de worktrees, prune, eliminación de ramas y repair tienen efectos persistentes. Esperar a Git puede dejar una operación pendiente si un hook o proceso bloquea, pero matarlo automáticamente puede interrumpir la mutación.

No se recomienda extender el timeout de lectura a estas operaciones. Sí conviene documentar feedback, diagnóstico y eventual cancelación segura por operación. Los guards observados protegen primary/branch/dirty state/sesiones y revalidan la confirmación; no se encontró otro defecto fuerte de destrucción. La carrera con modificaciones externas tras la última comprobación sigue siendo un límite, no una pérdida observada.

### Esperas single-flight y presión del pool

Las esperas de lecturas compartidas pueden alcanzar 180 s. `spawn_blocking` evita ejecutar esas esperas directamente en el hilo principal, pero no elimina latencia de operaciones ni presión de CPU/IO/memoria. No equivale a “imposible que la UI se degrade”. Medir duración agregada y número de workers con dependencias lentas.

## 8. Xterm/WebGL: diagnóstico abierto, no causa resuelta

**Prioridad media de observabilidad; causa original no confirmada.**

- **Rutas:** `src/lib/marvis-terminal.ts:289-438`, cleanup en `src/components/TerminalSession.vue:753-771`; `scripts/xterm-debug-log-plugin.mjs` y tests asociados.
- **Lo establecido:** versiones inspeccionadas xterm 6.0.0 y addon-webgl 0.19.0; addon manager/disposables tienen guards idempotentes en rutas inspeccionadas. Las sondas previas en happy-dom no reprodujeron `Attempted to dispose unknown listener`.
- **Lo no establecido:** stack causal real, ausencia absoluta de doble dispose, culpabilidad de xterm o arreglo mediante actualización. Tampoco se verificaron canvases/contextos reales en WKWebView durante sleep/presión de memoria.
- **Recuperación actual:** pérdida real notificada por addon, dispose y addon nuevo; máximo tres recuperaciones; no se gasta un intento por una mera vuelta a visible. El addon inspeccionado espera aproximadamente tres segundos por restauración y Marvis añade un segundo antes del reintento visible. Son políticas derivadas del código, no tiempos medidos en el runtime.
- **Transformación de build:** mantiene el `throw` y evaluación de argumentos, incluido `JSON.stringify`, pero elimina los tres `console.log` de contexto `disposed?`/`size?`/`arr?`. Esas llamadas están en la rama anómala: eliminarlas no arregla la causa y reduce evidencia justo al fallar.
- **Recomendación:** conservar diagnóstico estructurado y acotado del fallo, sin imprimir payloads ni un dump ilimitado de listeners. Obtener stack completo en dev —el transform aplica a build— y reproducir interacción/unmount/context loss con la dependencia real antes de cambiar versiones.
- **Aceptación:** secuencia real de pérdida/restore/unmount, errores globales y stack capturado; sin addons/canvases acumulados; error no silenciado y contexto útil tanto en dev como en build.

La observación previa de un timer de restauración que sobrevive brevemente al disposal es una limitación de dependencia inspeccionada, no prueba de la causa del listener desconocido.

## 9. Evaluación crítica de los informes de los agentes

Esta tabla conserva la trazabilidad de **todos los candidatos originales**. Los IDs descartados se mantienen como registro, no como trabajo pendiente.

| Candidato | Veredicto final                                  | Corrección/razón                                                                                                                                             |
| --------- | ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| LS-01     | Confirmado, alcance matizado                     | Abort observado y ausencia de log; no “única falla posible”. Loguear no evita el abort.                                                                      |
| LS-02     | Confirmado como consecuencia                     | Troubleshooting contradicho por artefactos; depende de LS-01, no duplicar impacto.                                                                           |
| LS-03     | Confirmado, matizado                             | Stderr sin cap; paths no son vulnerabilidad automática; resumen no redacta.                                                                                  |
| LS-04     | Clasificación confirmada; consecuencia pendiente | EPERM no prueba muerte. Condición de liveness propuesta inicialmente estaba invertida.                                                                       |
| LS-05     | Hueco confirmado; remedio inicial rechazado      | Webview target tiene dirección backend → frontend, no frontend → archivo.                                                                                    |
| LS-06     | Matizado                                         | Toast efímero y promesa que sigue; cierre no necesariamente inmediato. Orden de llamadas no prueba persistencia.                                             |
| LS-07     | Confirmado                                       | Catch sin guards/política. Mismo string no implica rerender; `syncStatusPolling` solo no detiene un estado running.                                          |
| PM-01     | Coste confirmado, importancia no medida          | Normalización por scroll; porcentajes/frecuencias no medidos.                                                                                                |
| PM-02     | Ruta confirmada, optimización por medir          | JSON por evento; no asumir paste de un único evento ni recomendar RAF/base64 por defecto.                                                                    |
| PM-03     | Riesgo no medido                                 | Mutex SQLite simple no demuestra cuello de botella ni exige WAL/pool.                                                                                        |
| PM-04     | **Descartado**                                   | `services/git.rs:1751-1755,2145-2153` vacía patch para large/tooLarge; `createHunks` exige `@@`. No construye miles de `DiffFile` por ese camino.            |
| PM-05     | Matizado                                         | Medidas geométricas repetidas, no forced reflow demostrado.                                                                                                  |
| PM-06     | Reformulado                                      | No renderiza en cada actividad; sí puede dejar imágenes obsoletas por el early return.                                                                       |
| PM-07     | Confirmado y acotado                             | Doble materialización/detección completa en Markdown; no tercera copia probada ni coste en todos los lenguajes.                                              |
| PM-08     | Matizado                                         | UTF-16 no son bytes/heap; corregir contrato, no extrapolar V8 a WKWebView.                                                                                   |
| PM-09     | Riesgo no medido                                 | Catálogo finito; LRU con dispose puede romper consumidores compartidos.                                                                                      |
| PM-10     | Matizado y precisado                             | El `finally` no tiene ownership. Una clave compuesta sola no basta. Guards de publicación sí existen.                                                        |
| PM-11     | Confirmado                                       | Wakeup acotado no implica payload acotado; puede acumular mientras worker bloquea.                                                                           |
| PM-12     | Coste de escalado derivable                      | Parse por listener; no evidencia que amerite nuevo dispatcher hoy.                                                                                           |
| PM-13     | **Descartado como problema grande**              | `DocumentPane.vue:99,927-956` usa compact source sobre 5000 líneas; el computed Set no se lee en esa rama. CodeMirror usa rangos.                            |
| PM-14     | **Premisa de duplicado descartada**              | `MainPane.vue:162-197` pasa path null al documento oculto; `changed-lines.ts:47-53,71-75` evita ese fetch; diff y documento no generan el duplicado alegado. |
| PM-15     | **Descartado**                                   | El comentario dice “at the fast pace”: 10 × 60/0,75 = 800/min es el contrafactual correcto. A 5 s son 120/min.                                               |

Otras afirmaciones que no se incorporan como evidencia:

- Cero `TODO`/`FIXME` no demuestra ausencia de deuda, dead code o bugs.
- Contar declaraciones de tests no sustituye ejecutar la suite ni demuestra cobertura de escenarios.
- Single-flight comparte por clave/revisión, no una única consulta universal para checkouts distintos.
- Un thread de drain no usa necesariamente `read_to_end` acotado; ver RV-07.
- Una compilación exitosa no prueba el arranque. Un rechazo de esquema correcto tampoco vuelve correcto el abort/diagnóstico.

## 10. Mantenibilidad: valoración y fronteras recomendadas

### Lo que está bien

- **Coordinadores concretos:** `diff-loader`, `source-highlight`, `changed-lines` y `layout-persistence` separan responsabilidades comprobables. Mantener `DocumentPane.loadFile` dentro del componente evita una abstracción genérica para estado estrechamente acoplado.
- **Tests de semántica:** generations, dispose tardío, bytes de terminal y coalescing son propiedades más útiles que contar líneas o snapshots indiscriminados.
- **Políticas explícitas:** nivel/rotación de logs, deadlines, fail-open, límites de virtualización y saves describen decisiones. El problema es cuando el comentario promete una invariante que el flujo no prueba, como PGID o bytes reales.
- **Persistencia sin ladder pre-1.0:** un único esquema; defaults humanos de YAML y tolerancia de API externa no equivalen a compatibilidad de formatos propios antiguos.
- **Errores de envío incierto en review:** se observaron transiciones condicionadas en DB y reconciliación del marker; no borrar la incertidumbre para aparentar éxito.

### Deuda concreta, no refactor por tamaño

1. **Varias semánticas de runner Git:** un módulo tiene deadlines y pipes; otro `.output()`; errores por stderr frente a stdout; diferentes interpretaciones de exit status. Es una frontera real para reutilizar ejecución/diagnóstico sin mezclar lecturas con mutaciones.
2. **Ownership de recursos asíncronos:** cada `finally`/callback debe poder demostrar que limpia su propio vuelo/recurso. Aplicar ese principio a PM-10 y cleanup de procesos, no crear una biblioteca genérica de coordinadores.
3. **Mapa de invalidación:** texto Markdown, imágenes, snapshot Git, líneas cambiadas y estado de agente tienen identidades diferentes. Documentarlas y probar dependencias evita tanto refresh duplicado como vistas obsoletas.
4. **Contratos de presupuesto:** distinguir mensajes, paths, caracteres, bytes, procesos y duración. Nombrar una variable `bytes` no basta; un canal de capacidad uno no acota los datos fusionados.
5. **Estado incierto observable:** post-rename sync fallido, cierre con writes pendientes y review enviado sin confirmación no deben convertirse en éxito/fallo simplista. La UI y el log deben explicar cuál fue el punto confirmado.

No se recomienda dividir `services/git.rs`, `App.vue` o componentes solo por su longitud —parte de la longitud corresponde a tests—. Si una implementación futura necesita extraer el runner, hacerlo por la frontera de ejecución y lifetime, con tests preservados y sin introducir dependencias innecesarias.

## 11. Estado factual de verificaciones

### 11.1 Verificaciones anteriores a esta revisión

| Verificación                | Resultado disponible                                 | Límite de lo demostrado                                                                                          |
| --------------------------- | ---------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Typecheck/lint frontend     | Reportados verdes                                    | No reejecutados en esta auditoría de solo lectura.                                                               |
| Vitest completo             | **816 tests** reportados verdes                      | Principalmente pruebas aisladas/mocks; no rendimiento de WKWebView real.                                         |
| Rust completo               | **357 pasan, 7 ignorados**, tres corridas reportadas | No se ejecutó Linux ni fallos de recursos/PGID en esta revisión.                                                 |
| Clippy/format/version check | Reportados verdes; manifiestos en **0.17.1**         | No prueban corrección de invariantes de cleanup.                                                                 |
| `pnpm build`                | Frontend/artefactos, exit 0                          | **No** equivale a `pnpm build:app` empaquetado. Hubo warnings de chunks/licencias instaladas, no error de build. |
| `pnpm test:artifacts`       | **17 pasan**, exit 0 observado                       | Tests de scripts/artefactos, no arranque de app.                                                                 |
| Compilación `pnpm dev:app`  | Rust compila                                         | El proceso luego aborta por DB incompatible.                                                                     |
| App dev                     | **Falla en setup, SIGABRT**                          | No hay sesión funcional validada.                                                                                |
| Bridge MCP                  | Inicialización observada antes de la caída           | No hubo sesión de driver funcional ni pruebas reales de UI por MCP.                                              |
| Release empaquetado         | **No realizado**                                     | Startup, logs y comportamiento release pendientes.                                                               |

Esta fase no reejecutó suites ni builds, ni lanzó la app. Su comprobación nueva es el análisis del árbol y la revisión crítica; el formato del documento puede verificarse por separado.

### 11.2 Deuda de validación que no debe cerrarse con mocks

- Xterm real: stack del listener, mouse reporting, selection, disposal, cambio de workspace y pérdida de WebGL.
- PTY: salida sostenida, ACKs perdidos, parser detenido, ocultación, sleep/resume y cierre bajo presión.
- Unix: grupos con distintos permisos, zombies y pertenencia/identidad antes y después de reap; Linux además de macOS.
- Watchers: worker lento y muchas rutas únicas; límite de memoria y refresh conservador tras overflow.
- Git: errores fatales frente a respuestas ausentes; paths/config no legibles; lectores fuera del runner endurecido.
- Persistencia: budgets de cierre y sync tras rename; distinguir estado confirmado de incierto.
- Empaquetado: build completa, launch gráfico, ruta de logs y diagnóstico de rechazo esperado.

## 12. Plan de medición y criterios de cierre

No se han recogido FPS, latencias p95, RSS sostenida o perfiles comparables antes/después. El siguiente protocolo produce evidencia para decidir optimizaciones sin trabajo especulativo:

| Escenario                                         | Métricas                                                                  | Qué permite decidir                                                    |
| ------------------------------------------------- | ------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| Scroll con 0/300/2000 directorios expandidos      | Tiempo de handler, normalizaciones, long tasks, CPU                       | PM-01/PM-05; especializar parche/consolidar medidas solo si aporta     |
| Terminal: teclado, paste grande, 1/10/30 sesiones | Latencia tecla→PTY, tamaño/cantidad de IPC, CPU de listeners, throughput  | PM-02/PM-12; no intercambiar throughput por mala respuesta interactiva |
| Salida PTY sostenida y suspensión                 | RSS nativa/webview, deuda, ACKs, tiempo de recuperación, bytes entregados | Garantías reales y coste del fail-open                                 |
| Muchos archivos/lenguajes                         | Memoria del webview, gramáticas vivas, entradas/promises del cache        | PM-08/PM-09; no usar una cifra V8 como medición WKWebView              |
| Markdown grande y cambio solo de imagen           | Tiempo de render/edición, número de lecturas de recursos, frescura        | PM-06/PM-07; evitar CPU extra y assets obsoletos                       |
| Build masivo con worker Git lento                 | Paths pendientes, bytes acumulados, RSS, procesos y refresh final         | PM-11/RV-07; presupuesto real, no solo límite de wakeups               |
| Operaciones concurrentes de UI/terminal/review    | Espera y hold time del mutex DB, duración de consultas/IPC                | PM-03; no rediseñar DB sin contención observada                        |

Usar el runtime objetivo y registrar fixtures, versión, plataforma y condiciones. Repetir escenarios comparables; separar caché fría/caliente. Las pruebas de rendimiento no deben almacenar contenido sensible ni transformarse en logging permanente por cada tecla/evento.

Un hallazgo se cierra con **corrección + prueba del fallo anterior + verificación del resultado observable**. Los de performance necesitan además medición comparable; los de plataforma, ejecución en la plataforma correspondiente. Hasta entonces, el estado correcto es “pendiente”, no “todos los puntos resueltos”.

## 13. Recomendación final

Conservar las mejoras previas y la simplicidad del proyecto. Primero corregir semánticas/ownership/diagnóstico, después medir rutas calientes y únicamente entonces optimizar. No introducir migraciones en v0, no ocultar excepciones para reducir ruido, no desactivar validación para ganar throughput y no implementar los candidatos descartados.

**Resultado de la evaluación inicial:** base de código mejorada, con huecos concretos todavía abiertos y verificación runtime incompleta. El seguimiento de remediación posterior está en §14; no reescribe lo que se observó en la evaluación inicial.

## 14. Seguimiento de remediación punto por punto

Estado tras recorrer los hallazgos del informe y revisar las correcciones aplicadas. “Corregido en código” describe lo cubierto por código/tests; **no** equivale a validar runtime, rendimiento o plataformas que no se ejecutaron. Este seguimiento complementa y, para el estado actual, prevalece sobre los pendientes redactados en las secciones anteriores.

| IDs                                   | Estado actual                       | Resultado y límite restante                                                                                                                                                                                                                                                                                                                                                                                             |
| ------------------------------------- | ----------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| LS-01 / LS-02                         | Parcial                             | `main.rs` registra una vez el paso y causa de `Database::open`/inicialización antes de propagar el error. No cambia el fallo de `setup` que puede acabar en `SIGABRT`; falta validar log y salida en app empaquetada. No se borró ni modificó la DB y no se alteró `README.md`.                                                                                                                                         |
| LS-03 / LS-08                         | Corregido en código                 | Los sinks usan resumen acotado (hasta 3 líneas, 120 caracteres por línea), prefieren stderr y recurren a stdout; el diagnóstico de `worktree repair` también queda acotado. **No redacta paths**.                                                                                                                                                                                                                       |
| LS-04                                 | Parcial; riesgo abierto             | Fuera de macOS, `EPERM` queda como no verificado. En macOS se conserva `EPERM → Gone` para el caso zombie que reproduce el test; no se puede distinguir de forma fiable un miembro vivo no señalable con las APIs usadas. Las pruebas no cubren un miembro privilegiado vivo; no se afirma resuelto.                                                                                                                    |
| LS-05 / LS-06                         | Parcial; runtime pendiente          | Se añadió un comando Rust de categorías cerradas y sin texto libre, captura frontend con rate limit y registro categorizado de deadlines de cierre. En release el error arbitrario no se envía ni se registra; el contexto de Vue solo va a consola en desarrollo. El cierre espera como máximo 250 ms por el diagnóstico: si IPC no responde, no se garantiza que llegue al archivo. Sin validación empaquetada/fsync. |
| LS-07                                 | Corregido en código/tests           | Polling coalescido, guards para respuestas tardías y checkout/sesión, errores transitorios recuperables y Retry para sesión inexistente/ownership; reintento terminal a 5 s.                                                                                                                                                                                                                                            |
| PM-01                                 | Cambio aplicado; medir impacto      | El patch de scroll evita renormalizar directorios expandidos ya validados y conserva validación del valor entrante. No hay perfil de WKWebView ni mejora porcentual medida.                                                                                                                                                                                                                                             |
| PM-02 / PM-03 / PM-05 / PM-09 / PM-12 | Pendientes de medición              | No se cambiaron transporte de input, conexión SQLite, lecturas geométricas, lifetime de gramáticas ni dispatcher de flujo. No hay perfiles comparables que justifiquen esas optimizaciones.                                                                                                                                                                                                                             |
| PM-04 / PM-13 / PM-14 / PM-15         | Descartados                         | Se mantienen los veredictos de §9; no se implementaron cambios para esos falsos positivos/premisas descartadas.                                                                                                                                                                                                                                                                                                         |
| PM-06                                 | Corregido en código/tests           | La actividad lleva paths; se refrescan solo imágenes Markdown usadas y afectadas, con invalidación completa conservadora ante overflow. Single-flight conserva eventos durante carga/refresco y evita publicar mapas concurrentes obsoletos. Persisten límites de validación runtime.                                                                                                                                   |
| PM-07                                 | Corregido para detección visual     | La detección lee el prefijo de líneas CodeMirror y limita el escaneo de fences sin alterar el contenido guardado. En un front matter sin cierre se omite decoración más allá del límite; no es límite de archivo ni de guardado.                                                                                                                                                                                        |
| PM-08                                 | Contrato corregido                  | El presupuesto se expresa en unidades UTF-16 (`string.length`), no bytes ni heap. Eviction sigue sin perfil de coste.                                                                                                                                                                                                                                                                                                   |
| PM-10                                 | Corregido en código/tests           | Vuelos del highlighter tienen ownership por checkout/path y cola propia; un `finally` obsoleto no limpia ni drena el vuelo actual.                                                                                                                                                                                                                                                                                      |
| PM-11                                 | Corregido en código/tests           | Payload watcher limita paths y bytes por checkout (50.000 / 4 MiB); ante overflow retiene actividad y solicita refresh completo (`paths: []`). No se midieron RSS ni coste real de `check-ignore`.                                                                                                                                                                                                                      |
| RV-01                                 | Corregido en código/tests           | Lecturas Git identificadas en workspace/worktree usan deadline de 30 s; operaciones mutantes siguen sin timeout deliberadamente. Falta probar dependencias Git colgadas en todas las plataformas.                                                                                                                                                                                                                       |
| RV-02 / RV-03                         | Corregido en código/tests           | Las refs distinguen ausencia esperada de error; `show-toplevel` solo clasifica como plain con respuesta reconocida. Se fija `LC_ALL=C` para ese diagnóstico. Se probaron casos Git locales/macOS; Linux/Windows pendientes.                                                                                                                                                                                             |
| RV-04 / RV-05                         | Corregido en código/tests           | Fallos al crear threads y de `try_wait` activan cleanup/reap; la señal al grupo solo se usa con identidad reservada y el fallo se propaga, no se convierte en salida vacía.                                                                                                                                                                                                                                             |
| RV-06                                 | Parcial; fuga excepcional abierta   | `waitid(WNOWAIT)` protege contra señalizar un PGID reutilizado y el fallback tras perder identidad solo termina el hijo conocido. Un helper que haga `setsid` y herede un pipe aún puede dejar el lector bloqueado en `read()` indefinidamente; cancelarlo portablemente requiere otra estrategia de I/O. El test de helper escapado no demuestra ausencia de fuga permanente.                                          |
| RV-07                                 | Corregido en código/tests           | stdout se limita a 16 MiB y stderr a 1 MiB; el overflow produce error en vez de éxito truncado y el lector sigue drenando para evitar bloquear Git. No se midió RSS en cargas reales; aplica el riesgo residual de helper escapado de RV-06.                                                                                                                                                                            |
| Xterm                                 | Diagnóstico mejorado; causa abierta | El transform de build conserva el contexto de la rama que lanza el error como un `console.error` acotado, manteniendo el throw. Tests del transform pasan; no se reprodujo el error real en WKWebView ni se probó el binario empaquetado.                                                                                                                                                                               |

### Verificaciones del seguimiento

| Comando                                                                                        | Resultado                                                                                                               |
| ---------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `pnpm typecheck && pnpm lint && pnpm exec vitest run`                                          | Exit 0; 60 archivos, 848 tests pasaron.                                                                                 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml --quiet`                             | Exit 0; 394 pasaron, 7 ignorados.                                                                                       |
| `cargo fmt --manifest-path src-tauri/Cargo.toml --check && pnpm lint:rust && git diff --check` | Exit 0.                                                                                                                 |
| `pnpm build`                                                                                   | Exit 0; typecheck, Vite y avisos de terceros. Permanecen warnings de chunks grandes y textos de licencia no instalados. |
| `node --test scripts/xterm-debug-log-plugin.test.mjs`                                          | Exit 0; 3 tests pasaron.                                                                                                |
| `node scripts/release.mjs --check`                                                             | Exit 0; todas las versiones siguen en `0.17.1`.                                                                         |

No se ejecutó `build:app`/launch empaquetado ni se abrió una DB incompatible: por eso siguen pendientes el abort de startup, la persistencia efectiva del log, xterm/WebGL real, PTY bajo presión, grupos con permisos adversos y pruebas Linux/Windows. No hubo borrado de DB, migración, cambio de versión, commit ni push.

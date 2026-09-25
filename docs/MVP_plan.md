# Marvis — Plan de base y primer MVP

> **OBSOLETO — sustituido por [PLAN_PENDIENTE.md](PLAN_PENDIENTE.md).** Se conserva como registro del plan original; para trabajo nuevo y criterios de cierre usar el plan de pendientes junto con [MVP_VALIDATION.md](MVP_VALIDATION.md). El alcance normativo sigue siendo `MARVIS_DELIVERY_PLAN_v0.4.md` y los requisitos v0.2.

> Plan de ejecución para el primer hito usable de la Entrega 1 (_The Workstation_). Se lee junto con `MARVIS_DELIVERY_PLAN_v0.4.md` y los requisitos v0.2, actualmente conservados en `MARVIS_REQUIREMENTS.md.old`. Si hay diferencias de modelo o alcance, prevalece v0.4.

**Estado:** obsoleto; no usar para planificar trabajo nuevo. **Plataforma inicial:** macOS.

## 1. Propósito y límites

Este documento divide la Entrega 1 en una base técnica, un MVP simple y el trabajo necesario para completarla. **MVP significa un hito interno, no una reducción del producto:** siguen vigentes las tres entregas y todos los requisitos del plan v0.4.

El primer recorrido que debe funcionar de extremo a extremo es:

```text
Abrir carpeta o checkout → identificar repo y checkout → abrir terminal real
→ modificar un archivo → ver estado y diff contra la rama por defecto
→ cerrar y reabrir sin perder la organización
```

No se requiere OpenCode para ese recorrido. La integración de agentes y review comienza en la Entrega 2, una vez cerrada la Entrega 1. No se implementan integraciones por adelantado solo para llenar estructuras vacías.

### Reglas que no se posponen

- `Repo` agrupa; `Checkout` es el contexto de trabajo; `Session` pertenece a un checkout. El inspector deriva siempre del checkout activo.
- Un checkout primario y uno creado con worktree tienen las mismas capacidades; un repo `plain` tiene exactamente un checkout y no ofrece funciones Git.
- Identidad por ruta canónica: abrir otra vez el mismo checkout lo enfoca, aunque se llegue por subdirectorio o symlink.
- Al abrir un worktree se resuelve su repo y se listan **todos** sus worktrees, no solo el abierto.
- El diff Git se compara siempre con la rama por defecto del repo, desde el merge-base, más los cambios sin confirmar. La referencia se muestra en el encabezado.
- Rust controla procesos, Git, PTY y archivos; Vue presenta el estado; SQLite persiste datos propios de Marvis. El WebView no obtiene una API genérica de shell ni acceso arbitrario al disco.

## 2. Orden y puertas de avance

| Fase         | Resultado verificable                                                         | Depende de                      |
| ------------ | ----------------------------------------------------------------------------- | ------------------------------- |
| 0. Riesgos   | Spikes de Git y terminal medidos; decisiones bloqueantes registradas          | Scaffold instrumental mínimo    |
| 1. Base      | App abre, identifica y restaura repos/checkouts; IPC y persistencia funcionan | Resultados de fase 0 aplicables |
| 2. MVP       | Recorrido completo con terminal, archivos y cambios                           | Base estable                    |
| 3. Entrega 1 | Cumple D1-01 a D1-20 y SEC-01 a SEC-07                                        | MVP usable                      |
| Después      | Entrega 2 (agentes y review), luego Entrega 3                                 | Entrega anterior cerrada        |

El scaffold instrumental para medir los spikes puede construirse al principio, pero **no** se da por definitiva la arquitectura de terminal o Git antes de superar sus pruebas. No hace falta terminar toda la fase 0 antes de preparar tipos, pruebas e infraestructura independientes de esos resultados.

## 3. Fase 0 — Despejar riesgos

### 0.1 Spike Git: repos y checkouts

Construir una prueba pequeña sobre Git CLI, con argumentos separados, que obtenga la raíz del repo, el checkout contenedor, la lista de worktrees, rama/HEAD y rama por defecto. Probar, como mínimo:

1. Abrir checkout primario, worktree con repo aún no abierto y subdirectorio de cualquiera de ellos.
2. Repetir las aperturas usando symlinks: el resultado debe tener la misma identidad canónica.
3. Worktree bajo `.worktrees/`, carpeta eliminada externamente, HEAD detached y repo sin commits.
4. Repositorio sin remoto y sin rama `main`: detectar la rama inicial o solicitar una elección **una sola vez** y recordarla; nunca usar una referencia incorrecta sin avisar.
5. Rama ya ocupada por otro checkout; submódulos; evaluar si un repo bare se admite o se rechaza con un mensaje claro.
6. Medir `status --porcelain=v2` en repos pequeños, medianos y grandes. Los presupuestos iniciales de v0.4 son <100 ms (<5k archivos), <300 ms (5k–50k), <1 s (50k–200k); siempre fuera del hilo de UI.

**Salida:** tabla de casos con entrada, resultado esperado/observado y tiempos; regla implementable para la rama por defecto y manejo de fallos. Si Git no permite resolver algún caso de forma fiable, decidir el comportamiento explícito antes de incorporar ese caso al servicio.

### 0.2 Spike terminal: PTY y renderer

Probar `portable-pty` → Rust → `Tauri Channel<ArrayBuffer>` → renderer WebView, separando `TerminalBackend` de la representación gráfica. Ejercitar `yes` durante 30 segundos, un `cat` grande, búsqueda con `rg`, `cargo build`, `pnpm install`, `nvim`, `htop`, cambios rápidos de tamaño y cinco terminales simultáneas.

Medir latencia de entrada, caudal de salida, pérdida de bytes, tiempo de resize, CPU, RAM y fluidez de la UI. **Pase:** ningún byte perdido, UI receptiva bajo `yes`, `nvim` utilizable y CPU en reposo cercana a cero. Si falla, ajustar flujo/chunks o evaluar otro renderer antes de construir la experiencia de sesiones encima. El flujo del PTY no se modela como miles de eventos globales de interfaz.

### 0.3 Decisiones antes de programar funciones dependientes

| Tema                           | Propuesta para avanzar                                                                                                               | Cierre necesario                                                                                                                                                                                        |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Q1: ubicación de worktrees     | Preferir un directorio externo al repo, configurable, para no modificar `.git/info/exclude` ni mostrar `.worktrees/` como untracked. | **Aprobar antes del spike Git definitivo y de crear worktrees.** El plan v0.4 propone `<repo>/.worktrees/<name>` como valor inicial, pero deja Q1 abierta: esta propuesta no lo reemplaza sin decisión. |
| Q11: rama por defecto incierta | Solicitar selección una vez y persistirla por repo.                                                                                  | Confirmar el comportamiento en el spike Git.                                                                                                                                                            |
| Q7/Q8: submódulos y bare       | Documentar resultado del spike y dar un comportamiento claro, sin fingir que son un checkout ordinario si no lo son.                 | Antes de cerrar resolución de rutas.                                                                                                                                                                    |

## 4. Fase 1 — Base técnica

### 4.1 Aplicación y estructura

- Inicializar Tauri 2 + Rust y Vue 3 + TypeScript + Vite. Preparar Tailwind y la base de Reka UI/shadcn-vue; añadir componentes concretos al necesitarlos.
- Organizar Rust por `domain`, `services`, `commands`, `terminal`, `git` y `persistence`, tomando la sección 46 de requisitos como guía y **renombrando Directory a Repo/Checkout** donde corresponda.
- Separar en Vue componentes de sidebar, sesiones e inspector, un cliente tipado de IPC, tipos de dominio y estado de presentación. No copiar el antiguo store `directories` con el modelo obsoleto.
- Incorporar tareas repetibles de desarrollo: arranque, formato, lint, typecheck, tests Rust y tests frontend. Ejecutarlas en CI desde que exista código comprobable.

**Comprobación:** la aplicación arranca en macOS, muestra un estado vacío útil y permite abrir una carpeta de prueba sin errores de IPC.

### 4.2 Modelo y propiedad del estado

- Definir `Repo { id, kind, name, root, defaultBranch?, checkouts, createdAt, lastOpenedAt }`, `Checkout { id, repoId, path, canonicalPath, isPrimary, branch?, head?, aheadOfDefault?, changedFiles, sessions }` y `Session` con identidad, tipo, checkout propietario y metadatos restaurables.
- Mantener por separado `activeCheckoutId` y `activeSessionId`; seleccionar una sesión no cambia la pertenencia de otras sesiones. `agent` queda reservado como tipo, sin simular un agente en el MVP.
- Canonicalizar las rutas antes de comparar identidades. Al abrir un subdirectorio, localizar el checkout contenedor con Git como fuente de verdad; no crear nodos para subdirectorios.
- Definir estados de carga/error para operaciones asíncronas y un bus de actividad limitado a los eventos usados en la Entrega 1.

**Comprobación:** pruebas unitarias para repo plain, primario/worktree, identidad repetida y transición entre checkouts; el repo no puede poseer sesiones directamente.

### 4.3 IPC, servicios y seguridad desde el inicio

- Commands de dominio estrechos para abrir/listar repos, estado/diff Git y operaciones de terminal. Channels solo para streams; validar IDs y pertenencia al checkout en cada operación.
- `RepoService`/`CheckoutService` resuelven rutas; `GitService` ejecuta Git fuera del hilo UI con arrays de argumentos; `TerminalService` es dueño del proceso PTY. No exponer `shell.exec(string)` ni permisos de shell/filesystem indiscriminados al WebView.
- Al leer archivos, canonicalizar el destino y rechazar symlinks que salgan del checkout. Mantener los diálogos de elección de carpeta como entrada controlada y validar otra vez en Rust.
- Crear un modelo de error con códigos y mensajes accionables: carpeta ausente, no es repo, rama por defecto desconocida, checkout/ID inválido, proceso terminado, Git falló.

**Comprobación:** pruebas con `..`, symlinks externos, nombres con espacios/metacaracteres y un ID que no corresponde al checkout; una llamada desde el WebView no puede lanzar un proceso arbitrario.

### 4.4 SQLite y restauración

- Crear migración inicial para repos registrados, orden, checkouts descubiertos, selección activa, metadatos de sesión, recientes y preferencias mínimas. Guardar IDs estables y rutas canónicas; no guardar referencia de diff por checkout.
- Al arrancar, recargar los repos registrados, volver a consultar Git para reconciliar worktrees y marcar como `Missing` los directorios desaparecidos. La lista persistida no sustituye a `git worktree list --porcelain`.
- Restaurar la organización visible y metadatos, **no afirmar que un PTY de una ejecución anterior sigue vivo**. Etiquetar las sesiones previas como inactivas/terminadas y permitir iniciarlas de nuevo; no almacenar salida terminal indefinidamente.
- Preparar migraciones versionadas para layout, drafts y demás datos de futuras fases, sin construir tablas o servicios de agente prematuramente.

**Comprobación:** cerrar y abrir la app mantiene repos, orden y selección; no duplica checkouts ni crea sesiones falsamente activas.

## 5. Fase 2 — MVP simple y usable

### 5.1 Abrir carpetas y navegar la jerarquía

Desde un diálogo de carpeta, comando o acción de interfaz, resolver primero la ruta canónica. Si Git detecta un checkout, obtener el repo primario y todos sus worktrees; si no, crear un repo `plain` con un checkout. Enfocar el checkout al que pertenece la ruta solicitada. Volver a abrir la misma ruta, un subdirectorio suyo o un symlink solo cambia el foco.

Renderizar `Repo → Checkout → Session`; incluir siempre el checkout primario, incluso vacío. Seleccionar un repo expande/colapsa; seleccionar checkout actualiza sesiones e inspector. Mostrar rama o estado básico y `Missing` si el directorio ya no existe. Un repo plain no muestra pestaña Changes.

**Comprobación:** casos D1-01 a D1-04; dos worktrees del mismo repo aparecen bajo un único repo, aunque el primero abierto sea el worktree.

### 5.2 Terminal que permite trabajar

Implementar `TerminalBackend` y su primera implementación con `portable-pty`; el renderer WebView se monta detrás de un adaptador. Una sesión se crea con el directorio del checkout, shell/entorno heredados y tamaño inicial correcto. Soportar varias sesiones por checkout, elegir cuál se ve, entrada/salida binaria por Channel, scrollback, resize y fin de proceso visible. Una sesión cerrada con proceso en marcha pide confirmación.

Para el MVP no hacen falta todavía splits anidados, pero el modelo de sesiones no debe asumir que siempre habrá una sola vista. Probar Unicode, color, pantalla alternativa, ratón y pegado con programas interactivos; no dar el spike por superado únicamente porque se vea un prompt.

**Comprobación:** iniciar dos shells en checkouts diferentes, ejecutar `nvim`, redimensionar y seguir escribiendo; un proceso que termina deja su código/estado visible en vez de desaparecer silenciosamente.

### 5.3 Archivos e inspector inicial

Mostrar un árbol navegable dentro del checkout activo, con selección de archivo y lectura controlada de texto. Respetar límites de seguridad de rutas, ignorar `.git` y aplicar reglas `.gitignore` donde corresponda. Al cambiar de checkout, no mantener el inspector apuntando a archivos del anterior. Diferenciar estados vacío, cargando, sin permisos y archivo eliminado.

En este hito basta una lista/árbol sencillo si el tamaño es manejable; la virtualización, búsqueda difusa, decoraciones Git completas y previews ricos se completan en fase 3. Archivos grandes o binarios deben tener un límite de lectura y un mensaje claro, nunca congelar la app.

**Comprobación:** seleccionar un archivo distinto en cada checkout muestra la ruta y contenido correctos; un symlink que escape del checkout se rechaza.

### 5.4 Estado Git y Changes

Detectar la rama por defecto en el orden de v0.4: `origin/HEAD`, rama inicial del repo y, si no se puede resolver, elección única persistida. Leer rama/HEAD y `git status --porcelain=v2`; ejecutar todo de forma asíncrona. Calcular el diff de commits desde el merge-base con la rama por defecto y presentar encima los cambios staged/unstaged sin duplicarlos; hacer visibles los archivos untracked como cambios pendientes, con tratamiento explícito para binarios. Mostrar `vs <rama>`.

En el MVP: lista de archivos cambiados y diff unificado bajo demanda por archivo, con líneas y estados de carga/error. Un cambio hecho desde la terminal debe reflejarse sin pulsar Refresh mediante watcher acotado por checkout, con debounce/coalescing; completar virtualización y límites avanzados en fase 3. La barra de estado indica rama y número de archivos cambiados; el ahead count puede añadirse al completar la Entrega 1.

**Comprobación:** editar un archivo desde terminal actualiza Changes automáticamente; los commits añadidos a la rama por defecto después de la divergencia no aparecen como cambios propios; repos plain no ejecutan comandos Git.

### 5.5 Persistencia y acciones mínimas

Ofrecer `Abrir directorio` y `Nueva terminal` desde controles visibles; si se añade una paleta reducida, esas dos acciones deben estar allí. Persistir repos, orden, checkout/sesión seleccionados y metadatos. En el reinicio se recupera la estructura y se distingue claramente una sesión histórica de un proceso vivo.

**Salida del MVP, demostración completa:** abrir un repo con worktrees preexistentes; navegar entre ellos; abrir y redimensionar una terminal con `nvim`; modificar un archivo; ver estado y diff sin salir de Marvis; cerrar y volver a abrir conservando la organización. Repetir apertura con subdirectorio/symlink sin duplicados y abrir una carpeta sin Git con archivos y terminal.

## 6. Fase 3 — Completar la Entrega 1

Este trabajo **no desaparece** por no estar dentro del primer MVP. Priorizarlo por dependencia:

1. **Worktrees:** crear desde cualquier checkout Git partiendo de la rama por defecto; solicitar nombre de tarea/rama editable, registrar y enfocar el nuevo checkout y abrir shell allí. Si la rama está ocupada, nombrar el checkout que la usa. Eliminar solo con salvaguardas para cambios sin confirmar, commits sin integrar, sesiones activas y checkout primario; tratar entradas obsoletas y decidir Q6 (conservar o eliminar rama).
2. **Terminal y layout:** pestañas, splits horizontales/verticales anidados, resize correcto para cada PTY, arrastre/orden de sesiones, restauración del layout, tratamiento de salida inesperada y sesiones `shell`, `nvim`, `server`, `custom`.
3. **Archivos y Git:** árbol virtualizado, búsqueda difusa, decoraciones, watcher eficiente, commits ahead, diff por archivo con hunks plegables y resaltado, viewed progress, límites y virtualización de diffs de 4.000+ líneas; impedir bloqueo de UI en repos grandes.
4. **Inspector y preview:** pestañas Files/Changes/Preview según capacidades; Markdown CommonMark/GFM con tablas, tareas, código e imágenes relativas bajo reglas de seguridad; texto y código. Neutralizar scripts, handlers y URLs peligrosas.
5. **Editores y ergonomía:** `EditorAdapter` para Zed y Neovim con detección y salto exacto a archivo/línea; paleta ⌘K completa, barra de estado completa y estados visuales de sesión verde/rojo/ninguno. Blue/amber corresponden a la Entrega 2.
6. **Actividad y robustez:** definir Q12 (qué cuenta como actividad), indicar actores concurrentes en sidebar/barra, directorios `Missing` con opciones Locate/Close, restauración de geometría y preferencias, revisión de mensajes de error, seguridad y empaquetado macOS.

**Puerta de salida:** ejecutar y documentar D1-01 a D1-20, incluidas carga de `yes` durante 30 s, `nvim` tras resize, diff grande, worktree eliminado fuera de la app, actividad concurrente, restauración y CPU en reposo. Automatizar SEC-01 a SEC-07 en CI. No declarar completa la Entrega 1 solo porque pase la demostración del MVP.

## 7. Después del MVP y la Entrega 1

- **Entrega 2 — Agent Loop:** spike de la interfaz actual de OpenCode, `AgentBridge`, estados y eventos, creación worktree→agente, Follow Agent, atención multiagente, comentarios de review y envío en lote, reconexión y permisos. Validar D2-01 a D2-15. No depender de scraping del PTY como fuente de verdad del agente.
- **Entrega 3 — Depth:** reanclaje de comentarios, previews de imagen/SVG/JSON/YAML, Neovim RPC, evaluación de libghostty, diff lado a lado, stage por hunk, layout responsive, accesibilidad y programa de benchmarks. Validar D3-01 a D3-10.

## 8. Checklist transversal de aceptación

- [ ] Cada hito deja un flujo demostrable y pruebas repetibles, no solo componentes sin conectar.
- [ ] Las operaciones Git, I/O de archivos y PTY nunca bloquean el hilo de UI.
- [ ] Rust valida rutas, IDs y pertenencia; procesos externos usan argumentos separados.
- [ ] Nunca se pierde la distinción entre checkout primario y worktree ni entre repo Git y carpeta plain.
- [ ] Los procesos no se restauran ficticiamente; Git y los worktrees se reconcilian al abrir.
- [ ] Las decisiones pendientes se registran antes de implementar las funciones que dependen de ellas; en particular, Q1 antes de crear worktrees.
- [ ] Las funcionalidades diferidas siguen enumeradas y se cierran en la entrega indicada por v0.4.

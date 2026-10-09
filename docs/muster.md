# Muster

Muster es una aplicación de escritorio para trabajar con repositorios git y sus worktrees. Está pensada para quien trabaja con varios agentes de consola sobre el mismo repositorio y necesita saber, de un vistazo, qué está haciendo cada uno, en qué rama, y qué cambió exactamente.

Es, si se quiere, una terminal con poderes. No es un IDE, ni pretende serlo.

## Por qué existe

La herramienta nació de un flujo de trabajo concreto: dejar a un agente haciendo una tarea mientras uno mismo se dedica a otra.

Un caso típico: un agente toma una tarea de Monday, crea el worktree, hace el trabajo, abre el merge request y avisa. Todo eso ocurre mientras el desarrollador está metido en otra cosa. El problema aparece cuando no es uno, sino tres o cuatro agentes en paralelo. A ellos se suman las sesiones de OpenCode que el propio desarrollador lanza para otras tareas, cada una con su worktree. El resultado es un conjunto de frentes abiertos que hay que vigilar a la vez: cuál terminó, cuál se detuvo, cuál necesita algo.

Antes de Muster, el ciclo de revisión era manual:

1. Notar que un agente se detuvo.
2. Ir a su directorio.
3. Correr la aplicación y probarla en una terminal.
4. Si algo estaba mal, volver al agente y explicárselo con palabras sueltas.
5. O abrir la merge request en el navegador, comentar allá y pegar los comentarios de vuelta en la terminal del agente.

El último paso era el más caro: la conversación sobre el código vivía en la herramienta de merge requests y el agente no la leía. Muster la trae a la pantalla, anotada sobre la línea exacta, y la devuelve al agente por su integración nativa o como Markdown para pegar donde haga falta. GitHub y GitLab no están integrados: el destino es OpenCode o el portapapeles.

El desorden no nace de la cantidad de trabajo, sino de que todo viva en el mismo cajón. Muster le da a cada frente el suyo, y junta en una sola pantalla lo que antes estaba repartido entre terminales, directorios y el navegador.

## Cómo se trabaja con Muster

La unidad de trabajo es el **checkout**: la raíz del repositorio o cualquiera de sus worktrees. Hay una sola ventana, y los checkouts de cada repositorio cuelgan de él en la barra lateral; al elegir uno se abre con sus archivos, sus diffs, sus terminales y sus notas de revisión. Nada se mezcla entre checkouts.

- **Terminales agrupadas por checkout.** Cada agente y sus terminales de apoyo (por ejemplo, la que corre el servidor de pruebas) viven juntos bajo su checkout. Al moverse entre terminales, el directorio de trabajo ya está en el lugar correcto.
- **Navegación pensada para teclado.** Moverse entre terminales, ramas y agentes es rápido, especialmente para quien ya vive en Neovim.
- **Detección automática de worktrees.** Si el usuario crea un worktree, Muster lo detecta. Si lo crea un agente, también.
- **Varios repositorios.** Se puede trabajar en otro repositorio o proyecto desde la misma ventana, cada uno con su propio grupo en la barra lateral.
- **Revisión sin salir de la pantalla.** Se ve qué agente trabaja en qué tarea, qué archivos cambió y qué falta revisar, y el veredicto se envía de vuelta al agente desde el mismo lugar.

## Barra lateral

Ordena el trabajo por repositorio. Bajo cada repositorio aparecen sus checkouts, y bajo cada checkout, sus terminales y sus agentes. Un agente no es una terminal anónima: es una sesión con ícono propio, rol y antigüedad.

Cada checkout muestra un balance de líneas agregadas y quitadas, y el número se calcula igual para todos, raíz incluida: lo que el checkout tiene de más o de menos que el punto donde divergió de la rama por defecto del repositorio, más lo que todavía está sin commitear en el working tree. No es "lo que cambió desde la última vez que se miró": responde a qué hizo el agente en total.

## Notas de revisión

Es el corazón de la herramienta. Se escriben sobre líneas específicas de un diff y recorren un ciclo:

1. **Borrador**
2. **Enviada**
3. **Resuelta**

Reemplazan la frase suelta ("el error es otro") por una coordenada: este archivo, esta línea, esto está mal. También reemplazan el viaje al navegador para comentar el merge request.

Cuando el código cambia y la nota deja de corresponder a lo que tiene debajo, Muster la marca como **obsoleta** en lugar de dejarla afirmando algo falso.

### Destinos de las notas

- **OpenCode (integración nativa):** sesiones, prompts y rondas de revisión con seguimiento, para no duplicar envíos.
- **Exportación a Markdown:** se guarda en un archivo que luego se pega en cualquier otro agente (Claude Code, Codex, pi, Hermes u otro) corrido por el usuario en una terminal dentro del checkout.

## Otras piezas

- Editor simple para ediciones rápidas.
- Varias terminales por checkout.
- Panel lateral con archivos (Files) y cambios (Changes).

## Lo que Muster no es

- No es un IDE.
- No es un harness de LLM.
- No es un cliente de GitHub o GitLab.
- No es una terminal a secas.

Es agnóstico respecto del agente: la integración con OpenCode es nativa, pero cualquier agente de consola funciona dentro de sus terminales.

## Para quién

Está orientado a quienes trabajan con Neovim y con agentes headless de consola, aunque cualquiera que tenga agentes corriendo en paralelo sobre worktrees puede usarlo.

## Resumen

Muster organiza la jornada de trabajo cuando hay varios agentes en marcha: permite ver qué hace cada uno, en qué rama, auditar lo que produjo con notas sobre líneas concretas, y seguir con lo propio sin confundir instancias, código ni tareas.

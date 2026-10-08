/** Media extensions describe presentation; Rust validates the bytes before assigning a MIME. */
export function mediaKind(path: string): "image" | "video" | "svg" | null {
  const extension = path.split(".").at(-1)?.toLowerCase();
  if (extension === "svg") return "svg";
  if (["png", "jpg", "jpeg", "gif", "webp", "avif", "ico", "bmp"].includes(extension ?? "")) return "image";
  if (["mp4", "webm", "mov", "ogv"].includes(extension ?? "")) return "video";
  return null;
}

export function svgBlob(source: string): Blob {
  const xml = new DOMParser().parseFromString(source, "image/svg+xml");
  const root = xml.documentElement;
  if (
    xml.querySelector("parsererror") ||
    root.localName !== "svg" ||
    root.namespaceURI !== "http://www.w3.org/2000/svg"
  ) {
    throw new Error("This file is not a valid SVG document.");
  }
  return new Blob([source], { type: "image/svg+xml" });
}

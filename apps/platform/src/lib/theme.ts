// Light is Soft White paper, dark the Midnight Navy ground; the system appearance picks, live.
export function followSystemTheme(root: HTMLElement, query: MediaQueryList): () => void {
  const apply = () => root.classList.toggle("dark", query.matches);
  apply();
  query.addEventListener("change", apply);
  return () => query.removeEventListener("change", apply);
}

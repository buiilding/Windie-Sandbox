// format.js — render model/user text as markdown, the windie-inspector way.
//
// Two rules, no sanitizer library:
//   1. Escape the source before parsing. Raw HTML becomes literal text, so a
//      model can never inject markup — the inspector's `skipHtml` equivalent.
//   2. Links: http/https/mailto/relative only, always new tab. Matches the
//      inspector's urlTransform + target=_blank behavior.
//
// Everything else is GFM markdown via `marked` (code fences, tables, lists,
// bold). Newlines follow the library default (soft wrap), same as the
// inspector's ReactMarkdown with no options.

// What this file gives you:
//   renderMarkdown(text)  ->  safe HTML string for innerHTML use.

// Escape HTML so raw tags in the source render as literal text.
function escapeHtml(text) {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

// One URL rule: only these schemes (or relative paths) are linkable.
function isAllowedUrl(url) {
  if (!url) return false;
  const lower = url.trim().toLowerCase();
  if (lower.startsWith("#") || lower.startsWith("/")) return true;
  return (
    lower.startsWith("http://") ||
    lower.startsWith("https://") ||
    lower.startsWith("mailto:")
  );
}

// Renderer: default GFM, with links forced to new tab and URL-filtered.
function makeRenderer() {
  const renderer = new marked.Renderer();
  const defaultLink = renderer.link.bind(renderer);
  renderer.link = (href, title, text) => {
    if (!isAllowedUrl(href)) return text; // strip link, keep visible text
    const html = defaultLink(href, title, text);
    return html.replace(
      /^<a /,
      '<a target="_blank" rel="noreferrer" '
    );
  };
  return renderer;
}

const renderer = makeRenderer();

// Public: markdown text -> safe HTML. Empty input -> empty string.
export function renderMarkdown(text) {
  return marked.parse(escapeHtml(text || ""), { renderer });
}

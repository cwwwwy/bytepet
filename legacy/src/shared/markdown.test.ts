/**
 * Security-critical tests for the Markdown pipeline.
 *
 * vitest runs in the `node` environment (vite.config.ts), where DOMPurify has no
 * DOM and reports `isSupported === false`. `renderMarkdown` therefore falls back
 * to the always-on `stripDangerousHtml` guard, which is the layer these tests
 * exercise. The DOMPurify branch is additionally covered by an injected
 * sanitizer so the composition order (guard first, sanitizer second) is pinned.
 */

import { describe, expect, it, vi } from "vitest";
import {
  clearMarkdownCache,
  domPurifySanitizer,
  escapeHtml,
  plainTextPreview,
  renderMarkdown,
  sanitizeHtml,
  stripDangerousHtml,
} from "./markdown";

describe("renderMarkdown sanitization", () => {
  it("strips <script> elements and their content", () => {
    const html = renderMarkdown('<script>alert("pwned")</script>\n\n# hi');
    expect(html).not.toMatch(/<script/i);
    expect(html).not.toContain("pwned");
    expect(html).toContain("<h1>hi</h1>");
  });

  it("strips event handler attributes from raw HTML", () => {
    const html = renderMarkdown('<img src="x" onerror="alert(1)" onload=alert(2)>');
    expect(html).not.toMatch(/onerror/i);
    expect(html).not.toMatch(/onload/i);
    expect(html).not.toContain("alert(1)");
  });

  it("neutralizes javascript: and vbscript: URLs in links and images", () => {
    const link = renderMarkdown("[click](javascript:alert(1))");
    expect(link.toLowerCase()).not.toContain("javascript:");
    const raw = renderMarkdown('<a href="JaVaScRiPt:alert(1)">x</a>');
    expect(raw.toLowerCase()).not.toContain("javascript:");
    const vb = renderMarkdown('<img src="vbscript:msgbox(1)">');
    expect(vb.toLowerCase()).not.toContain("vbscript:");
    const data = renderMarkdown('<iframe src="data:text/html,<script>alert(1)</script>"></iframe>');
    expect(data).not.toMatch(/<iframe/i);
  });

  it("strips style blocks and inline style attributes", () => {
    const html = renderMarkdown('<style>body{background:url("javascript:alert(1)")}</style>\n\nok');
    expect(html).not.toMatch(/<style/i);
    expect(html).not.toMatch(/style=/i);
    expect(html).toContain("ok");
  });

  it("keeps safe formatting, links and code fences", () => {
    const html = renderMarkdown("# Title\n\n**bold** and [link](https://example.com)\n\n```ts\nconst a = 1;\n```");
    expect(html).toContain("<h1>Title</h1>");
    expect(html).toContain("<strong>bold</strong>");
    expect(html).toContain('href="https://example.com"');
    expect(html).toContain('target="_blank"');
    expect(html).toContain("rel=\"noopener noreferrer nofollow\"");
    expect(html).toContain('class="md-copy"');
    expect(html).toContain("const a = 1;");
  });

  it("escapes code block contents instead of executing them", () => {
    const html = renderMarkdown("```html\n<script>alert(1)</script>\n```");
    expect(html).toContain("&lt;script&gt;");
    expect(html).not.toMatch(/<script/i);
  });

  it("preserves dangerous-looking text inside code blocks", () => {
    const html = renderMarkdown(
      '```html\n<img src=x onerror="alert(1)">\n<a href="javascript:alert(1)">x</a>\n```',
    );
    expect(html).toContain("onerror=");
    expect(html).toContain("javascript:");
    expect(html).toContain("&lt;img");
    expect(html).not.toMatch(/<img/i);
  });

  it("escapes unsafe characters when rendering attributes", () => {
    expect(escapeHtml('<b class="x">&\'')).toBe("&lt;b class=&quot;x&quot;&gt;&amp;&#39;");
  });

  it("runs the guard before the injected sanitizer", () => {
    const spy = vi.fn((html: string) => html);
    clearMarkdownCache();
    renderMarkdown('<script>alert(1)</script>', { sanitizer: spy, noCache: true });
    expect(spy).toHaveBeenCalledTimes(1);
    const passed = spy.mock.calls[0][0];
    expect(passed).not.toMatch(/<script/i);
  });

  it("exposes a fallback sanitizer that works without a DOM", () => {
    expect(typeof window).toBe("undefined");
    const output = domPurifySanitizer('<b>ok</b><script>alert(1)</script>');
    expect(output).toContain("<b>ok</b>");
    expect(output).not.toMatch(/<script/i);
  });

  it("sanitizeHtml composes the guard with the sanitizer", () => {
    const sanitizer = (html: string) => html.replace(/<b>/g, "");
    expect(sanitizeHtml("<b>hi</b>", sanitizer)).toBe("hi</b>");
  });
});

describe("stripDangerousHtml", () => {
  it("handles unquoted event handlers", () => {
    expect(stripDangerousHtml("<img src=x onerror=alert(1)>")).not.toMatch(/onerror/i);
  });

  it("handles uppercase attributes and tags", () => {
    expect(stripDangerousHtml("<IMG SRC=x ONERROR=alert(1)>")).not.toMatch(/onerror/i);
  });

  it("handles a '>' inside another quoted attribute", () => {
    const out = stripDangerousHtml('<img alt="a > b" onerror="alert(1)">');
    expect(out).not.toMatch(/onerror/i);
    expect(out).toContain('alt="a > b"');
  });

  it("handles a trailing unclosed tag", () => {
    expect(stripDangerousHtml("<img src=x onerror=alert(1)")).not.toMatch(/onerror/i);
  });

  it("removes srcdoc and style attributes", () => {
    expect(stripDangerousHtml('<iframe srcdoc="<script>x</script>"></iframe>')).not.toMatch(/srcdoc/i);
    expect(stripDangerousHtml('<div style="background:url(javascript:1)">x</div>')).not.toMatch(/style=/i);
  });

  it("handles whitespace/entity obfuscated javascript: urls", () => {
    expect(stripDangerousHtml('<a href="java\tscript:alert(1)">x</a>')).not.toMatch(/javascript/i);
    expect(stripDangerousHtml("<a href='JAVASCRIPT:alert(1)'>x</a>")).not.toMatch(/javascript/i);
  });

  it("leaves escaped code samples untouched", () => {
    const input = '&lt;img onerror="alert(1)"&gt; &lt;a href="javascript:x"&gt;';
    expect(stripDangerousHtml(input)).toBe(input);
  });

  it("leaves ordinary markup untouched", () => {
    const input = '<p class="a">hello <code>x</code></p>';
    expect(stripDangerousHtml(input)).toBe(input);
  });

  it("stays fast on large documents", () => {
    const block = "# heading\n\nSome **bold** text with a [link](https://example.com).\n\n```ts\nconst x = 1;\n```\n\n";
    const started = Date.now();
    const html = renderMarkdown(block.repeat(400));
    expect(html.length).toBeGreaterThan(10_000);
    expect(Date.now() - started).toBeLessThan(3000);
  });
});

describe("plainTextPreview", () => {
  it("removes markdown noise and truncates", () => {
    expect(plainTextPreview("# Title\n\n- item `code`")).toBe("Title item code");
    expect(plainTextPreview("x".repeat(50), 10)).toBe(`${"x".repeat(10)}…`);
  });
});

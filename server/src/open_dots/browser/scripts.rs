//! Page-side scripts the dot's browser evaluates with `Runtime.evaluate` as
//! `(<source>)(...<json args>)`. They are JavaScript on purpose: they run inside the page. The
//! find-by-text and find-field logic is ported from Open Dot's `computer/dom-actions.ts`
//! (`clickScript`, `typeScript`), the login fill from its `cdp-helper.ts`, and the snapshot from
//! Ghostex's own `ghostexSnapshotScript` (`ghostex_cli/browser_mcp/page_scripts.rs`).
//!
//! No script ever returns the value of a password field, nor of a field `FILL_LOGIN` filled with a
//! saved password (marked `data-gx-dot-secret`, so it stays hidden after a "Show password" button
//! turns it into a text field). They run in an isolated world (see `session::Page::run`), so page
//! scripts cannot patch the functions they call.

/// Stamps `data-gx-dot-ref="e<n>"` on every listed element and returns `{ url, title, total,
/// elements: [{ ref, role, label, value, filled, checked, disabled, href, inView, selector }] }`.
/// Elements in the viewport come first.
pub(super) const SNAPSHOT: &str = r##"function gxDotSnapshot(limit) {
  const ATTR = "data-gx-dot-ref";
  for (const old of document.querySelectorAll("[" + ATTR + "]")) old.removeAttribute(ATTR);
  const selectors = [
    "a[href]", "button", "input:not([type=hidden])", "textarea", "select", "summary",
    "[role=button]", "[role=link]", "[role=tab]", "[role=menuitem]", "[role=option]",
    "[role=checkbox]", "[role=radio]", "[role=switch]", "[role=textbox]", "[role=combobox]",
    "[contenteditable='']", "[contenteditable=true]", "[onclick]", "[tabindex]:not([tabindex='-1'])",
  ].join(",");
  const clean = (s) => (s || "").replace(/\s+/g, " ").trim();
  const cssPath = (element) => {
    const parts = [];
    let cursor = element;
    while (cursor && cursor.nodeType === 1 && cursor !== document.documentElement) {
      let part = cursor.nodeName.toLowerCase();
      if (cursor.id) {
        parts.unshift(part + "#" + CSS.escape(cursor.id));
        break;
      }
      const parent = cursor.parentElement;
      if (parent) {
        const siblings = Array.from(parent.children).filter((child) => child.nodeName === cursor.nodeName);
        if (siblings.length > 1) part += ":nth-of-type(" + (siblings.indexOf(cursor) + 1) + ")";
      }
      parts.unshift(part);
      cursor = parent;
    }
    return parts.join(" > ");
  };
  const isPassword = (el) => el.hasAttribute("data-gx-dot-secret") || (el.nodeName === "INPUT" && (el.getAttribute("type") || "").toLowerCase() === "password");
  const labelFor = (el) => {
    const aria = el.getAttribute("aria-label");
    if (aria) return clean(aria);
    const by = el.getAttribute("aria-labelledby");
    if (by) {
      const text = by.split(/\s+/).map((id) => document.getElementById(id)).filter(Boolean).map((n) => n.innerText).join(" ");
      if (clean(text)) return clean(text);
    }
    if (el.id) {
      const label = document.querySelector('label[for="' + CSS.escape(el.id) + '"]');
      if (label && clean(label.innerText)) return clean(label.innerText);
    }
    const wrap = el.closest("label");
    if (wrap && wrap !== el && clean(wrap.innerText)) return clean(wrap.innerText);
    const own = clean(el.innerText);
    if (own) return own;
    const type = (el.getAttribute("type") || "").toLowerCase();
    if (el.nodeName === "INPUT" && ["submit", "button", "reset"].includes(type) && !isPassword(el)) return clean(el.value);
    const img = el.querySelector && el.querySelector("img[alt]");
    return clean(el.getAttribute("placeholder") || el.getAttribute("title") || (img && img.getAttribute("alt")) || el.getAttribute("name") || "");
  };
  const roleFor = (el) => {
    const role = el.getAttribute("role");
    if (role) return role;
    const tag = el.nodeName.toLowerCase();
    const type = (el.getAttribute("type") || "text").toLowerCase();
    if (tag === "a") return "link";
    if (tag === "input") {
      if (isPassword(el)) return "password";
      if (["checkbox", "radio", "range", "file", "color", "date", "time", "datetime-local", "month", "week"].includes(type)) return type;
      if (["submit", "button", "reset", "image"].includes(type)) return "button";
      return "textbox";
    }
    if (tag === "textarea") return "textbox";
    if (el.isContentEditable) return "textbox";
    return tag;
  };
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const seen = [];
  for (const el of document.querySelectorAll(selectors)) {
    const r = el.getBoundingClientRect();
    const st = window.getComputedStyle(el);
    if (r.width <= 0 || r.height <= 0 || st.visibility === "hidden" || st.display === "none") continue;
    seen.push({ el, inView: r.bottom > 0 && r.right > 0 && r.top < vh && r.left < vw });
  }
  const ordered = seen.filter((s) => s.inView).concat(seen.filter((s) => !s.inView));
  const elements = [];
  for (const { el, inView } of ordered.slice(0, limit)) {
    const ref = "e" + (elements.length + 1);
    el.setAttribute(ATTR, ref);
    const tag = el.nodeName.toLowerCase();
    const type = (el.getAttribute("type") || "").toLowerCase();
    const password = isPassword(el);
    let value = "";
    if (!password && (tag === "textarea" || tag === "select" || (tag === "input" && !["checkbox", "radio", "submit", "button", "reset", "image", "file"].includes(type)))) {
      value = clean(String(el.value ?? "")).slice(0, 120);
    }
    let checked = null;
    if (tag === "input" && (type === "checkbox" || type === "radio")) checked = !!el.checked;
    else if (el.hasAttribute("aria-checked")) checked = el.getAttribute("aria-checked") === "true";
    elements.push({
      ref,
      role: roleFor(el),
      label: labelFor(el).slice(0, 120),
      value,
      filled: password ? String(el.value || "").length > 0 : null,
      checked,
      disabled: Boolean(el.disabled || el.getAttribute("aria-disabled") === "true"),
      href: tag === "a" ? String(el.href || "").slice(0, 200) : "",
      inView,
      selector: cssPath(el),
    });
  }
  return { url: location.href, title: document.title, total: seen.length, elements };
}"##;

/// Finds the element to click, by snapshot ref or by visible text (Open Dot's `clickScript`
/// matching: exact, then starts-with, then contains, then any leaf text), scrolls it into view
/// and marks it. Returns `{ ok, error?, label, x, y, hit }`; `hit` is true when a real mouse
/// click at `x,y` lands on it. With `dry` it only finds the element and returns `{ ok, label }`,
/// every name it goes by (text, value, aria-label, title, alt), for the risky-click check.
pub(super) const CLICK_TARGET: &str = r##"function gxDotClickTarget(mode, q, fallback, dry) {
  const MARK = "data-gx-dot-target";
  if (!dry) for (const old of document.querySelectorAll("[" + MARK + "]")) old.removeAttribute(MARK);
  const norm = (s) => (s || "").replace(/\s+/g, " ").trim().toLowerCase();
  const isPassword = (e) => e.hasAttribute("data-gx-dot-secret") || (e.nodeName === "INPUT" && (e.getAttribute("type") || "").toLowerCase() === "password");
  const text = (e) => e.innerText || (isPassword(e) ? "" : e.value) || e.getAttribute("aria-label") || e.getAttribute("title") || e.getAttribute("alt") || "";
  let el = null;
  if (mode === "ref") {
    el = document.querySelector('[data-gx-dot-ref="' + CSS.escape(q) + '"]');
    if (!el && fallback) { try { el = document.querySelector(fallback); } catch (_) { el = null; } }
    if (!el) return { ok: false, error: "Element " + q + " is not on the page any more. Take a new page_snapshot and use a fresh ref." };
  } else {
    const vis = (e) => { const r = e.getBoundingClientRect(); const st = getComputedStyle(e); return r.width > 0 && r.height > 0 && st.visibility !== "hidden" && st.display !== "none"; };
    const want = norm(q);
    const label = (e) => norm(text(e));
    const sel = 'button, a, [role=button], [role=link], [role=tab], [role=option], [role=menuitem], [role=checkbox], [role=radio], input[type=submit], input[type=button], input[type=checkbox], input[type=radio], label, summary, [onclick], [tabindex]';
    const all = [...document.querySelectorAll(sel)].filter(vis);
    const pickFrom = (list) => list.find((e) => label(e) === want) || list.find((e) => label(e).startsWith(want)) || list.find((e) => label(e).includes(want));
    el = want ? pickFrom(all) : null;
    if (!el && want) {
      const any = [...document.querySelectorAll("body *")].filter((e) => vis(e) && e.children.length === 0 && norm(e.textContent).includes(want));
      el = any[0] && (any[0].closest(sel) || any[0]);
    }
    if (!el) return { ok: false, error: "Nothing on the page matches " + JSON.stringify(q) + ". Read the page and use the exact text you see." };
  }
  if (dry) {
    // Every name the element goes by, so a button whose text and aria-label differ is judged by both.
    const names = [];
    for (const name of [el.innerText, isPassword(el) ? "" : el.value, el.getAttribute("aria-label"), el.getAttribute("title"), el.getAttribute("alt")]) {
      const cleaned = String(name || "").replace(/\s+/g, " ").trim();
      if (cleaned && !names.some((known) => known.toLowerCase() === cleaned.toLowerCase())) names.push(cleaned);
    }
    return { ok: true, label: names.join(" / ").slice(0, 200) };
  }
  el.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
  el.setAttribute(MARK, "1");
  const r = el.getBoundingClientRect();
  const x = r.left + r.width / 2;
  const y = r.top + r.height / 2;
  const inside = x >= 0 && y >= 0 && x < window.innerWidth && y < window.innerHeight;
  const at = inside ? document.elementFromPoint(x, y) : null;
  const hit = !!at && (at === el || el.contains(at));
  return { ok: true, label: text(el).replace(/\s+/g, " ").trim().slice(0, 80), x, y, hit };
}"##;

/// Clicks the element `CLICK_TARGET` marked with a DOM click (when a real mouse click would land
/// on something covering it).
pub(super) const CLICK_MARKED: &str = r##"function gxDotClickMarked() {
  const el = document.querySelector("[data-gx-dot-target]");
  if (!el) return false;
  el.click();
  return true;
}"##;

/// Finds a field by snapshot ref or by label/placeholder/name (Open Dot's `typeScript`
/// matching), focuses it and selects its content so typed text replaces it. Password fields are
/// refused. `<select>` elements are set here directly from `value` (option text or value).
/// Returns `{ ok, error?, field, kind: "text" | "editable" | "select" }`.
pub(super) const FOCUS_FIELD: &str = r##"function gxDotFocusField(mode, q, fallback, value) {
  const MARK = "data-gx-dot-target";
  for (const old of document.querySelectorAll("[" + MARK + "]")) old.removeAttribute(MARK);
  const norm = (s) => (s || "").replace(/\s+/g, " ").trim().toLowerCase();
  const names = (e) => {
    const byFor = e.id ? document.querySelector('label[for="' + CSS.escape(e.id) + '"]') : null;
    return [byFor && byFor.innerText, e.closest("label") && e.closest("label").innerText, e.getAttribute("aria-label"), e.getAttribute("placeholder"), e.getAttribute("name"), e.id].map(norm).filter(Boolean);
  };
  let el = null;
  if (mode === "ref") {
    el = document.querySelector('[data-gx-dot-ref="' + CSS.escape(q) + '"]');
    if (!el && fallback) { try { el = document.querySelector(fallback); } catch (_) { el = null; } }
    if (!el) return { ok: false, error: "Element " + q + " is not on the page any more. Take a new page_snapshot and use a fresh ref." };
  } else {
    const vis = (e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
    const want = norm(q);
    const fields = [...document.querySelectorAll("input:not([type=hidden]):not([type=password]):not([data-gx-dot-secret]), textarea, [contenteditable=''], [contenteditable=true], select")].filter(vis);
    const pick = (list) => list.find((e) => names(e).some((n) => n === want)) || list.find((e) => names(e).some((n) => n.includes(want)));
    el = pick(fields) || (want ? null : fields[0]);
    if (!el && want && pick([...document.querySelectorAll("input[type=password]")].filter(vis))) {
      return { ok: false, error: "That is a password field. Use sign_in to fill a saved login; never type passwords yourself." };
    }
    if (!el) return { ok: false, error: "No field matches " + JSON.stringify(q) + ". Read the page and use the field's label or placeholder." };
  }
  if (el.hasAttribute("data-gx-dot-secret") || (el.nodeName === "INPUT" && (el.getAttribute("type") || "").toLowerCase() === "password")) {
    return { ok: false, error: "That is a password field. Use sign_in to fill a saved login; never type passwords yourself." };
  }
  const field = names(el)[0] || el.nodeName.toLowerCase();
  el.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
  el.focus();
  if (el.nodeName === "SELECT") {
    const want = norm(value);
    const options = [...el.options];
    const option = options.find((o) => norm(o.text) === want || norm(o.value) === want) || options.find((o) => norm(o.text).includes(want));
    if (!option) return { ok: false, error: "The list " + JSON.stringify(field) + " has no option " + JSON.stringify(value) + ". Options: " + options.map((o) => o.text.trim()).slice(0, 30).join(", ") };
    el.value = option.value;
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { ok: true, field, kind: "select" };
  }
  const editable = !("value" in el) || el.isContentEditable;
  if (!editable && !(el.nodeName === "INPUT" || el.nodeName === "TEXTAREA")) {
    return { ok: false, error: "That element is not a text field." };
  }
  el.setAttribute(MARK, "1");
  if (editable) {
    const range = document.createRange();
    range.selectNodeContents(el);
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
    return { ok: true, field, kind: "editable" };
  }
  try { el.select(); } catch (_) {}
  return { ok: true, field, kind: "text" };
}"##;

/// Whether the marked field now holds `value` (ignoring the spaces and punctuation an input mask
/// adds); when it does not (a field that ignores inserted text), sets it with the native value
/// setter and input/change events. Returns `{ ok, value }`, `value` = what the field shows when
/// it still differs (never a password field: `FOCUS_FIELD` refuses those).
pub(super) const ENSURE_FIELD_VALUE: &str = r##"function gxDotEnsureFieldValue(value) {
  const el = document.querySelector("[data-gx-dot-target]");
  if (!el) return { ok: false, value: "" };
  const squash = (s) => String(s || "").replace(/[^\p{L}\p{N}]/gu, "").toLowerCase();
  const editable = el.isContentEditable || !("value" in el);
  const current = () => editable ? (el.innerText || el.textContent || "") : String(el.value ?? "");
  const matches = () => editable ? squash(current()).includes(squash(value)) : (current() === value || squash(current()) === squash(value));
  if (matches()) return { ok: true, value: "" };
  if (!editable) {
    const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, "value").set.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    if (matches()) return { ok: true, value: "" };
  }
  return { ok: false, value: current().replace(/\s+/g, " ").trim().slice(0, 120) };
}"##;

/// Fills the visible login form: the username/email field (preferring one in the password's
/// form) and the password field, with the native value setter and input/change events (Open
/// Dot's `cdp-helper.ts` fill). Refuses first unless the page is https and its host is `host` or a
/// subdomain of it (an empty `host` refuses). The filled password field is marked
/// `data-gx-dot-secret` and drawn masked even if the page later turns it into a text field.
/// Returns `{ refused?: "https" | "host", page, user, pass }`, never a value.
pub(super) const FILL_LOGIN: &str = r##"function gxDotFillLogin(u, p, host) {
  const here = String(location.hostname || "").toLowerCase().replace(/\.$/, "");
  if (location.protocol !== "https:") return { refused: "https", page: location.protocol === "data:" ? "a data: page" : (here || location.protocol), user: false, pass: false };
  const want = String(host || "");
  if (!want || !(here === want || here.endsWith("." + want))) return { refused: "host", page: here, user: false, pass: false };
  const vis = (e) => !!(e.offsetWidth || e.offsetHeight || e.getClientRects().length) && !e.disabled && !e.readOnly;
  const set = (el, v) => {
    el.focus();
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(el, v);
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  };
  const pass = [...document.querySelectorAll("input[type=password], input[data-gx-dot-secret]")].find(vis);
  const groups = [
    'input[type=email], input[autocomplete~=username], input[autocomplete~=email]',
    'input[name*=user i], input[name*=email i], input[name*=login i], input[id*=user i], input[id*=email i], input[id*=login i]',
    'input[type=text], input:not([type])',
  ];
  const find = (scope) => {
    for (const group of groups) {
      const hit = [...scope.querySelectorAll(group)].find((e) => vis(e) && e.type !== "password" && e.type !== "hidden" && !e.hasAttribute("data-gx-dot-secret"));
      if (hit) return hit;
    }
    return null;
  };
  const scope = pass && pass.form ? pass.form : null;
  const user = (scope && find(scope)) || find(document);
  let filledUser = false;
  if (user && u) { set(user, u); filledUser = user.value === u; }
  let filledPass = false;
  if (pass) {
    set(pass, p);
    filledPass = pass.value.length === p.length;
    pass.setAttribute("data-gx-dot-secret", "1");
    pass.style.setProperty("-webkit-text-security", "disc", "important");
  }
  return { page: here, user: filledUser, pass: filledPass };
}"##;

/// `{ url, title, ready }` of the page.
pub(super) const PAGE_INFO: &str = r##"function gxDotPageInfo() { return { url: location.href, title: document.title, ready: document.readyState }; }"##;

/// The page's visible text.
pub(super) const PAGE_TEXT: &str =
    r##"function gxDotPageText() { return document.body ? document.body.innerText : ""; }"##;

/// `{ y, height, viewport }` scroll position.
pub(super) const SCROLL_INFO: &str = r##"function gxDotScrollInfo() {
  const root = document.scrollingElement || document.documentElement;
  return { y: Math.round(window.scrollY), height: Math.round(root ? root.scrollHeight : 0), viewport: window.innerHeight };
}"##;

/* Aura-Emerge site — mobile nav, tabs, scroll active section, EN|UA i18n */

(function () {
  "use strict";

  var LANG_KEY = "aura-lang";

  function getLang() {
    try {
      var v = localStorage.getItem(LANG_KEY);
      if (v === "ua" || v === "en") return v;
    } catch (_) {}
    return "en";
  }

  function setLang(lang) {
    try {
      localStorage.setItem(LANG_KEY, lang);
    } catch (_) {}
    applyLang(lang);
  }

  function applyLang(lang) {
    document.documentElement.lang = lang === "ua" ? "uk" : "en";
    document.documentElement.setAttribute("data-lang", lang);

    document.querySelectorAll("[data-i18n]").forEach(function (el) {
      var key = el.getAttribute("data-i18n");
      var map = I18N[key];
      if (!map) return;
      var text = map[lang] != null ? map[lang] : map.en;
      if (text == null) return;
      if (el.tagName === "INPUT" || el.tagName === "TEXTAREA") {
        el.placeholder = text;
      } else if (el.hasAttribute("data-i18n-html")) {
        el.innerHTML = text;
      } else {
        el.textContent = text;
      }
    });

    document.querySelectorAll("[data-i18n-attr]").forEach(function (el) {
      var spec = el.getAttribute("data-i18n-attr");
      if (!spec) return;
      var parts = spec.split(":");
      if (parts.length < 2) return;
      var attr = parts[0];
      var key = parts.slice(1).join(":");
      var map = I18N[key];
      if (!map) return;
      var text = map[lang] != null ? map[lang] : map.en;
      if (text != null) el.setAttribute(attr, text);
    });

    document.querySelectorAll(".lang-block").forEach(function (el) {
      var blockLang = el.getAttribute("data-lang-block");
      el.hidden = blockLang !== lang;
    });

    document.querySelectorAll(".lang-switch .lang-btn").forEach(function (btn) {
      var active = btn.getAttribute("data-lang") === lang;
      btn.classList.toggle("active", active);
      btn.setAttribute("aria-pressed", active ? "true" : "false");
    });

    // Lang change can show/hide sections — refresh in-page nav highlight
    if (typeof initScrollSpy === "function") {
      // defer so layout has applied [hidden]
      setTimeout(function () { initScrollSpy(); }, 0);
    }
  }

  /* ── shared chrome strings ─────────────────────────────────────── */
  var I18N = {
    nav_overview: { en: "Overview", ua: "Огляд" },
    nav_features: { en: "Features", ua: "Можливості" },
    nav_install: { en: "Installation", ua: "Встановлення" },
    nav_quickstart: { en: "Quick start", ua: "Швидкий старт" },
    nav_world: { en: "World & sets", ua: "World і набори" },
    nav_config: { en: "make.conf", ua: "make.conf" },
    nav_usage: { en: "Usage", ua: "Використання" },
    nav_scanner: { en: "PKGBUILD scanner", ua: "Сканер PKGBUILD" },
    nav_sandbox: { en: "bwrap sandbox", ua: "Пісочниця bwrap" },
    nav_mask: { en: "package.mask", ua: "package.mask" },
    nav_links: { en: "Links", ua: "Посилання" },
    nav_docs: { en: "Docs", ua: "Документація" },
    nav_flags: { en: "Flags", ua: "Прапори" },
    nav_start: { en: "Start", ua: "Старт" },
    nav_core: { en: "Core", ua: "Ядро" },
    nav_security: { en: "Security", ua: "Безпека" },
    nav_more: { en: "More", ua: "Більше" },
    nav_pages: { en: "Pages", ua: "Сторінки" },
    btn_github: { en: "GitHub", ua: "GitHub" },
    btn_install: { en: "Install", ua: "Встановити" },
    btn_get_started: { en: "Get started", ua: "Почати" },
    btn_view_github: { en: "View on GitHub", ua: "На GitHub" },
    btn_aur: { en: "AUR package", ua: "Пакунок AUR" },
    menu_open: { en: "Open menu", ua: "Відкрити меню" },
    hero_badge: { en: "Arch · Portage-style · Security-first", ua: "Arch · у стилі Portage · безпека перш за все" },
    hero_lead: {
      en: "A standalone Gentoo-style emerge for Arch Linux — installs from official repos, the AUR, and ABS; scans PKGBUILDs for supply-chain attack patterns; and runs untrusted build steps inside a bwrap sandbox.",
      ua: "Автономний ПМ у стилі Gentoo emerge для Arch Linux — встановлює з офіційних репозиторіїв, AUR та ABS, сканує PKGBUILD на ознаки атак на ланцюг постачання і виконує недовірені кроки збірки всередині пісочниці bwrap."
    },
    features_title: { en: "Features", ua: "Можливості" },
    features_intro: {
      en: "Packages are tracked in a world file — install once, track forever. The tool drives pacman for official packages and builds AUR/ABS itself.",
      ua: "Пакунки відстежуються у файлі world — встановив один раз, відстежується назавжди. Інструмент керує pacman для офіційних пакунків і сам збирає AUR/ABS."
    },
    footer_source: { en: "Source", ua: "Код" },
    footer_pages: { en: "GitHub Pages", ua: "GitHub Pages" },
    not_aura: {
      en: "fosskers/aura? No — after v2.1, Aura is no longer part of this project. Aura-Emerge is a separate, security-first reimplementation.",
      ua: "fosskers/aura? Ні — після v2.1 Aura більше не є частиною цього проєкту. Aura-Emerge — окрема реалізація з пріоритетом безпеки."
    },
    docs_title: { en: "Documentation", ua: "Документація" },
    docs_lead: {
      en: "Full reference for world, make.conf, mask, sandbox, scanner, and everyday usage — the README in page form.",
      ua: "Повний довідник: world, make.conf, mask, пісочниця, сканер і повсякденне використання — README у вигляді сторінки."
    },
    flags_title: { en: "CLI flags", ua: "Прапори CLI" },
    flags_lead: {
      en: "Every flag accepted by emerge — actions, modifiers, Gentoo-compat no-ops, and generated completions/man page.",
      ua: "Усі прапори, які приймає emerge — дії, модифікатори, сумісність з Gentoo і згенеровані completions/man."
    },
    tab_aur: { en: "AUR", ua: "AUR" },
    tab_github: { en: "GitHub (Unstable)", ua: "GitHub (нестабільне)" },
    section_install: { en: "Installation", ua: "Встановлення" },
    section_quickstart: { en: "Quick start", ua: "Швидкий старт" },
    section_world: { en: "World & sets", ua: "World і набори" },
    section_config: { en: "make.conf", ua: "make.conf" },
    section_usage: { en: "Usage highlights", ua: "Основне використання" },
    section_scanner: { en: "Security: PKGBUILD scanner", ua: "Безпека: сканер PKGBUILD" },
    section_sandbox: { en: "bwrap sandbox", ua: "Пісочниця bwrap" },
    section_mask: { en: "package.mask", ua: "package.mask" },
    section_links: { en: "Links", ua: "Посилання" },
    binary_name: { en: "Binary name", ua: "Ім'я бінарника" },
    binary_name_body: {
      en: "The installed binary is emerge (with a portageq symlink). Completions and the man page are generated from the same CLI definition.",
      ua: "Встановлений бінарник — emerge (з symlink portageq). Completions і man-сторінка генеруються з того ж визначення CLI."
    },
    deps_req: { en: "Required:", ua: "Обов'язково:" },
    deps_opt: { en: "Optional:", ua: "Опційно:" },
    not_hard_block: { en: "Not a hard block", ua: "Не жорстке блокування" },
    not_hard_block_body: {
      en: "Findings are reported with file/line and a cgit link. You get Continue anyway? [y/N] — the decision stays with you.",
      ua: "Знахідки повідомляються з файлом/рядком і посиланням на cgit. Запит Continue anyway? [y/N] — рішення залишається за вами."
    }
  };

  window.AuraI18N = { getLang: getLang, setLang: setLang, applyLang: applyLang, I18N: I18N };

  /* ── sidebar (from original main.js) ───────────────────────────── */
  function initSidebar() {
    var toggle = document.getElementById("menu-toggle");
    var sidebar = document.getElementById("sidebar");
    var overlay = document.getElementById("sidebar-overlay");
    if (!toggle || !sidebar) return;

    function closeSidebar() {
      sidebar.classList.remove("open");
      if (overlay) {
        overlay.classList.remove("show");
        overlay.setAttribute("aria-hidden", "true");
      }
      document.body.classList.remove("nav-open");
    }

    function openSidebar() {
      sidebar.classList.add("open");
      if (overlay) {
        overlay.classList.add("show");
        overlay.setAttribute("aria-hidden", "false");
      }
      document.body.classList.add("nav-open");
    }

    toggle.addEventListener("click", function (e) {
      e.preventDefault();
      e.stopPropagation();
      if (sidebar.classList.contains("open")) closeSidebar();
      else openSidebar();
    });

    /* Backdrop only - sidebar is a sibling above it in z-index */
    if (overlay) {
      overlay.addEventListener("click", function (e) {
        e.preventDefault();
        closeSidebar();
      });
    }

    /* Extra safety: never let a drawer tap bubble to document */
    sidebar.addEventListener("click", function (e) {
      e.stopPropagation();
    });
    sidebar.addEventListener(
      "touchstart",
      function (e) {
        e.stopPropagation();
      },
      { passive: true }
    );

    sidebar.querySelectorAll("a").forEach(function (a) {
      a.addEventListener("click", function () {
        if (window.innerWidth <= 900) closeSidebar();
      });
    });
  }

  /* ── tabs - only direct children so nested groups stay independent ─ */
  function initTabs() {
    document.querySelectorAll("[data-tabs]").forEach(function (root) {
      var list = root.querySelector(":scope > .tabs-list");
      if (!list) return;
      var tabs = list.querySelectorAll(":scope > .tab");
      var panels = root.querySelectorAll(":scope > .tab-panel");

      tabs.forEach(function (tab) {
        tab.addEventListener("click", function (e) {
          e.stopPropagation();
          var id = tab.getAttribute("data-tab");
          tabs.forEach(function (t) {
            var on = t === tab;
            t.classList.toggle("active", on);
            t.setAttribute("aria-selected", on ? "true" : "false");
          });
          panels.forEach(function (p) {
            var on = p.getAttribute("data-panel") === id;
            p.classList.toggle("active", on);
            if (on) p.removeAttribute("hidden");
            else p.setAttribute("hidden", "");
          });
        });
      });
    });
  }

  /* ── highlight sidebar nav by scroll (in-page #anchors only) ───── */
  var scrollSpyObserver = null;

  function initScrollSpy() {
    var navLinks = document.querySelectorAll(".nav-link[href^='#']");
    if (!navLinks.length) return;

    if (scrollSpyObserver) {
      scrollSpyObserver.disconnect();
      scrollSpyObserver = null;
    }

    // Only sections that are actually laid out (not inside a hidden lang-block)
    var sections = Array.prototype.slice
      .call(document.querySelectorAll("main section[id]"))
      .filter(function (s) {
        return s.offsetParent !== null || s.getClientRects().length > 0;
      });
    if (!sections.length) return;

    function setActive(id) {
      navLinks.forEach(function (l) {
        l.classList.toggle("active", l.getAttribute("href") === "#" + id);
      });
    }

    // At top of page: highlight the first in-page section link
    function activateTopIfNeeded() {
      if (window.scrollY < 80) {
        setActive(sections[0].id);
      }
    }

    scrollSpyObserver = new IntersectionObserver(
      function (entries) {
        // Prefer the topmost intersecting section
        var visible = entries
          .filter(function (e) { return e.isIntersecting; })
          .sort(function (a, b) {
            return a.boundingClientRect.top - b.boundingClientRect.top;
          });
        if (visible.length) setActive(visible[0].target.id);
      },
      { rootMargin: "-15% 0px -55% 0px", threshold: 0 }
    );

    sections.forEach(function (s) {
      scrollSpyObserver.observe(s);
    });

    activateTopIfNeeded();
  }

  function initYear() {
    var el = document.getElementById("year");
    if (el) el.textContent = String(new Date().getFullYear());
  }

  function initLangSwitch() {
    document.querySelectorAll(".lang-switch .lang-btn").forEach(function (btn) {
      btn.addEventListener("click", function () {
        var lang = btn.getAttribute("data-lang");
        if (lang) setLang(lang);
      });
    });
    applyLang(getLang());
  }

  /* Mark current page in Pages nav (index / docs / flags) */
  function initActivePage() {
    var path = (location.pathname.split("/").pop() || "index.html").toLowerCase();
    if (!path || path === "") path = "index.html";
    document.querySelectorAll(".nav-link[data-page]").forEach(function (a) {
      var page = (a.getAttribute("data-page") || "").toLowerCase();
      if (page && page === path) a.classList.add("active");
      else if (page) a.classList.remove("active");
    });
  }

  document.addEventListener("DOMContentLoaded", function () {
    initSidebar();
    initTabs();
    initScrollSpy();
    initYear();
    initLangSwitch();
    initActivePage();
  });
})();
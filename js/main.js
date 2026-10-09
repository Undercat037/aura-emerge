/* Aura-Emerge site — mobile nav, tabs, scroll active section, EN|UA|RU i18n */

(function () {
  "use strict";

  var LANG_KEY = "aura-lang";

  function getLang() {
    try {
      var v = localStorage.getItem(LANG_KEY);
      if (v === "ua" || v === "en" || v === "ru") return v;
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
    document.documentElement.lang = lang === "ua" ? "uk" : (lang === "ru" ? "ru" : "en");
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
    nav_overview: { en: "Overview", ua: "Огляд", ru: "Обзор" },
    nav_features: { en: "Features", ua: "Можливості", ru: "Возможности" },
    nav_install: { en: "Installation", ua: "Встановлення", ru: "Установка" },
    nav_quickstart: { en: "Quick start", ua: "Швидкий старт", ru: "Быстрый старт" },
    nav_world: { en: "World & sets", ua: "World і набори", ru: "World и наборы" },
    nav_config: { en: "make.conf", ua: "make.conf", ru: "make.conf" },
    nav_usage: { en: "Usage", ua: "Використання", ru: "Использование" },
    nav_scanner: { en: "PKGBUILD scanner", ua: "Сканер PKGBUILD", ru: "Сканер PKGBUILD" },
    nav_sandbox: { en: "bwrap sandbox", ua: "Пісочниця bwrap", ru: "Песочница bwrap" },
    nav_mask: { en: "package.mask", ua: "package.mask", ru: "package.mask" },
    nav_links: { en: "Links", ua: "Посилання", ru: "Ссылки" },
    nav_docs: { en: "Docs", ua: "Документація", ru: "Документация" },
    nav_flags: { en: "Flags", ua: "Прапори", ru: "Флаги" },
    nav_start: { en: "Start", ua: "Старт", ru: "Старт" },
    nav_core: { en: "Core", ua: "Ядро", ru: "Ядро" },
    nav_security: { en: "Security", ua: "Безпека", ru: "Безопасность" },
    nav_more: { en: "More", ua: "Більше", ru: "Ещё" },
    nav_pages: { en: "Pages", ua: "Сторінки", ru: "Страницы" },
    btn_github: { en: "GitHub", ua: "GitHub", ru: "GitHub" },
    btn_install: { en: "Install", ua: "Встановити", ru: "Установить" },
    btn_get_started: { en: "Get started", ua: "Почати", ru: "Начать" },
    btn_view_github: { en: "View on GitHub", ua: "На GitHub", ru: "На GitHub" },
    btn_aur: { en: "AUR package", ua: "Пакунок AUR", ru: "Пакет AUR" },
    menu_open: { en: "Open menu", ua: "Відкрити меню", ru: "Открыть меню" },
    hero_badge: { en: "Arch · Portage-style · Security-first", ua: "Arch · у стилі Portage · безпека перш за все", ru: "Arch · в стиле Portage · безопасность прежде всего" },
    hero_lead: {
      en: "Gentoo-style emerge for Arch — packages from official repos, the AUR, and ABS. PKGBUILDs get scanned for supply-chain tricks; untrusted build steps run inside a bwrap sandbox.",
      ua: "Emerge у стилі Gentoo для Arch — пакунки з офіційних репо, AUR і ABS. PKGBUILD скануються на трюки ланцюга постачання; недовірені кроки збірки йдуть у пісочниці bwrap.",
      ru: "Emerge в стиле Gentoo для Arch — пакеты из официальных репозиториев, AUR и ABS. PKGBUILD сканируются на приёмы атак на цепочку поставок; ненадёжные шаги сборки выполняются в песочнице bwrap."
    },
    features_title: { en: "Features", ua: "Можливості", ru: "Возможности" },
    features_intro: {
      en: "Everything you install lands in a world file — install once, keep track forever. Official packages go through pacman; AUR and ABS are built by emerge itself.",
      ua: "Усе, що ставите, потрапляє у файл world — один раз встановив, далі відстежується. Офіційні пакунки через pacman; AUR і ABS збирає сам emerge.",
      ru: "Всё, что вы ставите, попадает в файл world — установил один раз, дальше отслеживается. Официальные пакеты идут через pacman; AUR и ABS собирает сам emerge."
    },
    footer_source: { en: "Source", ua: "Код", ru: "Исходники" },
    footer_pages: { en: "GitHub Pages", ua: "GitHub Pages", ru: "GitHub Pages" },
    not_aura: {
      en: "fosskers/aura? No — after v2.1, Aura is no longer part of this project. Aura-Emerge is a separate, security-first reimplementation.",
      ua: "fosskers/aura? Ні — після v2.1 Aura більше не є частиною цього проєкту. Aura-Emerge — окрема реалізація з пріоритетом безпеки.",
      ru: "fosskers/aura? Нет — после v2.1 Aura больше не часть этого проекта. Aura-Emerge — отдельная реализация с приоритетом безопасности."
    },
    docs_title: { en: "Documentation", ua: "Документація", ru: "Документация" },
    docs_lead: {
      en: "Full reference for world, make.conf, mask, sandbox, scanner, and everyday usage.",
      ua: "Повний довідник: world, make.conf, mask, пісочниця, сканер і повсякденне використання.",
      ru: "Конфигурация, world, наборы, сканер, песочница и остальное."
    },
    flags_title: { en: "CLI flags", ua: "Прапори CLI", ru: "Флаги" },
    flags_lead: {
      en: "Every flag accepted by emerge — actions, modifiers, Gentoo-compat no-ops, and generated completions/man page.",
      ua: "Усі прапори, які приймає emerge — дії, модифікатори, сумісність з Gentoo і згенеровані completions/man.",
      ru: "Полный список флагов командной строки."
    },
    tab_aur: { en: "AUR", ua: "AUR", ru: "AUR" },
    tab_github: { en: "GitHub (Unstable)", ua: "GitHub (нестабільна)", ru: "GitHub" },
    section_install: { en: "Installation", ua: "Встановлення", ru: "Установка" },
    section_quickstart: { en: "Quick start", ua: "Швидкий старт", ru: "Быстрый старт" },
    section_world: { en: "World & sets", ua: "World і набори", ru: "World и наборы" },
    section_config: { en: "make.conf", ua: "make.conf", ru: "make.conf" },
    section_usage: { en: "Usage highlights", ua: "Основне використання", ru: "Использование" },
    section_scanner: { en: "Security: PKGBUILD scanner", ua: "Безпека: сканер PKGBUILD", ru: "Сканер PKGBUILD" },
    section_sandbox: { en: "bwrap sandbox", ua: "Пісочниця bwrap", ru: "Песочница bwrap" },
    section_mask: { en: "package.mask", ua: "package.mask", ru: "package.mask" },
    section_links: { en: "Links", ua: "Посилання", ru: "Ссылки" },
    binary_name: { en: "Binary name", ua: "Назва виконуваного файлу", ru: "Имя бинарника" },
    binary_name_body: {
      en: "The installed binary is <code>emerge</code> (with a <code>portageq</code> symlink). Completions and the man page are generated from the same CLI definition.",
      ua: "Встановлюється виконуваний файл <code>emerge</code> (із символічним посиланням <code>portageq</code>). Автодоповнення та man-сторінка генеруються з того самого опису CLI.",
      ru: "Пакет ставит <code>emerge</code> в <code>/usr/bin/emerge</code> (и <code>portageq</code>). Это намеренно: CLI совместим с привычками Gentoo."
    },
    tab_manual: { en: "Manual", ua: "Вручну", ru: "Вручную" },
    qs_not_installed: { en: "Not installed yet?", ua: "Ще не встановили?", ru: "Ещё не установлено?" },
    qs_see_install: { en: "See the installation guide.", ua: "Перегляньте інструкцію зі встановлення.", ru: "См. раздел «Установка»." },
    deps_req: { en: "Required:", ua: "Обов'язково:", ru: "Зависимости" },
    deps_opt: { en: "Optional:", ua: "Опційно:", ru: "Опционально" },
    not_hard_block: { en: "Not a hard block", ua: "Не жорстке блокування", ru: "Не жёсткая блокировка" },
    not_hard_block_body: {
      en: "Findings are reported with file/line and a cgit link. You get Continue anyway? [y/N] — the decision stays with you.",
      ua: "Знахідки повідомляються з файлом/рядком і посиланням на cgit. Запит Continue anyway? [y/N] — рішення залишається за вами.",
      ru: "Срабатывание сканера не останавливает установку жёстко: вы видите файл, строку, ссылку на cgit и вопрос «Continue anyway? [y/N]»."
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

  /* ── highlight sidebar nav by scroll (in-page #anchors only) ─────
     Position-based, not IntersectionObserver: the active section is
     always recomputed from the current scroll offset, so scrolling up
     can never leave a stale highlight. */
  var spyBound = false;
  var spyTicking = false;

  function spyUpdate() {
    spyTicking = false;
    var navLinks = document.querySelectorAll(".nav-link[href^='#']");
    if (!navLinks.length) return;

    // Only sections that are actually laid out (not inside a hidden lang-block)
    var sections = Array.prototype.slice
      .call(document.querySelectorAll("main section[id]"))
      .filter(function (s) {
        return s.offsetParent !== null || s.getClientRects().length > 0;
      });
    if (!sections.length) return;

    var probe = window.innerHeight * 0.3;
    var current = sections[0];
    sections.forEach(function (s) {
      if (s.getBoundingClientRect().top <= probe) current = s;
    });
    // Short last sections never reach the probe line: pin to the end
    if (window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 2) {
      current = sections[sections.length - 1];
    }

    navLinks.forEach(function (l) {
      l.classList.toggle("active", l.getAttribute("href") === "#" + current.id);
    });
  }

  function requestSpy() {
    if (spyTicking) return;
    spyTicking = true;
    window.requestAnimationFrame(spyUpdate);
  }

  function initScrollSpy() {
    if (!spyBound) {
      spyBound = true;
      window.addEventListener("scroll", requestSpy, { passive: true });
      window.addEventListener("resize", requestSpy);
      window.addEventListener("hashchange", requestSpy);
    }
    spyUpdate();
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

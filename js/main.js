/* Aura-Emerge docs - mobile nav, tabs, active section */

(function () {
  const toggle = document.getElementById("menu-toggle");
  const sidebar = document.getElementById("sidebar");
  const overlay = document.getElementById("sidebar-overlay");

  function closeSidebar() {
    sidebar?.classList.remove("open");
    overlay?.classList.remove("show");
  }

  toggle?.addEventListener("click", () => {
    sidebar?.classList.toggle("open");
    overlay?.classList.toggle("show");
  });

  overlay?.addEventListener("click", closeSidebar);

  sidebar?.querySelectorAll("a").forEach((a) => {
    a.addEventListener("click", () => {
      if (window.innerWidth <= 900) closeSidebar();
    });
  });

  // Tabs - only direct children so nested tab groups stay independent
  document.querySelectorAll("[data-tabs]").forEach((root) => {
    const list = root.querySelector(":scope > .tabs-list");
    if (!list) return;
    const tabs = list.querySelectorAll(":scope > .tab");
    const panels = root.querySelectorAll(":scope > .tab-panel");

    tabs.forEach((tab) => {
      tab.addEventListener("click", (e) => {
        e.stopPropagation();
        const id = tab.getAttribute("data-tab");
        tabs.forEach((t) => {
          const on = t === tab;
          t.classList.toggle("active", on);
          t.setAttribute("aria-selected", on ? "true" : "false");
        });
        panels.forEach((p) => {
          const on = p.getAttribute("data-panel") === id;
          p.classList.toggle("active", on);
          if (on) p.removeAttribute("hidden");
          else p.setAttribute("hidden", "");
        });
      });
    });
  });

  // Highlight sidebar nav by scroll
  const sections = document.querySelectorAll("main section[id]");
  const navLinks = document.querySelectorAll(".nav-link[href^='#']");

  function setActive(id) {
    navLinks.forEach((l) => {
      l.classList.toggle("active", l.getAttribute("href") === "#" + id);
    });
  }

  const observer = new IntersectionObserver(
    (entries) => {
      entries.forEach((e) => {
        if (e.isIntersecting) setActive(e.target.id);
      });
    },
    { rootMargin: "-20% 0px -60% 0px", threshold: 0 }
  );

  sections.forEach((s) => observer.observe(s));

  const y = document.getElementById("year");
  if (y) y.textContent = new Date().getFullYear();
})();
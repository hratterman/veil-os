// ============================================================
// shared.js — Dark mode, page transitions, shared utilities.
// Include on every page BEFORE page-specific scripts.
// ============================================================

// ── DARK MODE ───────────────────────────────────────────────
(function initDarkMode() {
  // Apply theme ASAP to prevent flash
  const saved = localStorage.getItem('theme');
  if (saved === 'dark' || (!saved && window.matchMedia('(prefers-color-scheme: dark)').matches)) {
    document.documentElement.classList.add('dark');
  }
})();

function toggleDarkMode() {
  const isDark = document.documentElement.classList.toggle('dark');
  localStorage.setItem('theme', isDark ? 'dark' : 'light');
  updateThemeColor();
  updateToggleIcon();
}

function updateThemeColor() {
  const isDark = document.documentElement.classList.contains('dark');
  const meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.content = isDark ? '#141413' : '#2C4A2E';
}

function updateToggleIcon() {
  const btn = document.getElementById('theme-toggle');
  if (!btn) return;
  const isDark = document.documentElement.classList.contains('dark');
  btn.innerHTML = isDark
    ? '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="12" cy="12" r="5"/><path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42"/></svg>'
    : '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>';
  btn.setAttribute('aria-label', isDark ? 'Switch to light mode' : 'Switch to dark mode');
}

// ── PAGE TRANSITIONS ────────────────────────────────────────
function initPageTransitions() {
  // Fade in on load
  document.body.style.opacity = '0';
  document.body.style.transition = 'opacity 0.25s ease';
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      document.body.style.opacity = '1';
    });
  });

  // Intercept internal navigation links
  document.addEventListener('click', function(e) {
    const link = e.target.closest('a');
    if (!link) return;

    const href = link.getAttribute('href');
    if (!href) return;

    // Skip anchor links, external links, mailto, tel, downloads
    if (href.startsWith('#') || href.startsWith('mailto:') || href.startsWith('tel:')) return;
    if (link.target === '_blank') return;
    if (link.hasAttribute('download')) return;

    // Only handle internal links
    try {
      const url = new URL(href, window.location.origin);
      if (url.origin !== window.location.origin) return;
      if (url.pathname === window.location.pathname && url.hash) return;
    } catch { return; }

    e.preventDefault();
    document.body.style.opacity = '0';
    setTimeout(() => {
      window.location.href = href;
    }, 250);
  });
}

// ── CONSOLE MESSAGE ──────────────────────────────────────────
function initConsoleMessage() {
  console.log(
    '%cHR',
    'font-size: 32px; font-weight: bold; color: #2C4A2E; font-family: serif;'
  );
  console.log(
    '%cNo React. No Next. No Tailwind.\nHTML, CSS, JS. Hosted on a Mac Mini in my apartment.\n\nhenryratterman@gmail.com',
    'font-size: 11px; color: #666; line-height: 1.5;'
  );
}

// ── KONAMI CODE ─────────────────────────────────────────────
function initKonamiCode() {
  const code = ['ArrowUp','ArrowUp','ArrowDown','ArrowDown','ArrowLeft','ArrowRight','ArrowLeft','ArrowRight','b','a'];
  let pos = 0;
  document.addEventListener('keydown', function(e) {
    if (e.key === code[pos]) {
      pos++;
      if (pos === code.length) {
        pos = 0;
        activateKonami();
      }
    } else {
      pos = 0;
    }
  });
}

function activateKonami() {
  // Retro 8-bit mode
  document.documentElement.classList.add('retro-mode');

  // Show a fun message
  const msg = document.createElement('div');
  msg.className = 'konami-toast';
  msg.innerHTML = '↑↑↓↓←→←→BA';
  document.body.appendChild(msg);
  requestAnimationFrame(() => msg.classList.add('visible'));

  // Revert after 4 seconds
  setTimeout(() => {
    msg.classList.remove('visible');
    document.documentElement.classList.remove('retro-mode');
    setTimeout(() => msg.remove(), 500);
  }, 4000);
}

// ── HR LOGO RAPID CLICK → HOMESTARRUNNER ────────────────────
function initLogoEasterEgg() {
  const logo = document.querySelector('.nav-logo');
  if (!logo) return;
  let clicks = 0;
  let timer = null;

  logo.addEventListener('click', function(e) {
    clicks++;
    if (clicks >= 5) {
      e.preventDefault();
      clicks = 0;
      clearTimeout(timer);
      window.location.href = 'https://homestarrunner.com';
      return;
    }
    clearTimeout(timer);
    timer = setTimeout(() => { clicks = 0; }, 800);
  });
}

// ── INIT ────────────────────────────────────────────────────
function initShared() {
  updateThemeColor();
  updateToggleIcon();
  initPageTransitions();
  initConsoleMessage();
  initKonamiCode();
  initLogoEasterEgg();
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initShared);
} else {
  initShared();
}


// ============================================================
//  Rendering engine — reads from CONTENT and populates DOM.
//  You should never need to edit this file.
// ============================================================

function statusBadge(type, label) {
  if (!type) return label ? `<span class="status-footnote">${label}</span>` : '';
  if (type === 'live')     return `<div class="live-badge"><div class="live-dot"></div><span class="live-text">${label}</span></div>`;
  if (type === 'building') return `<div class="building-badge"><div class="building-dot"></div><span class="building-text">${label}</span></div>`;
  return `<span class="status-footnote">${label}</span>`;
}

function render() {
  const C = CONTENT;

  // ── Meta
  document.getElementById('page-title').textContent       = C.meta.title;
  document.getElementById('page-description').content     = C.meta.description;

  // ── Hero
  document.getElementById('hero-eyebrow').textContent     = C.hero.eyebrow;
  document.getElementById('hero-name').innerHTML          = `${C.hero.firstName}<br><em>${C.hero.lastName}</em>`;
  document.getElementById('hero-tagline').textContent     = C.hero.tagline;
  document.getElementById('hero-status').textContent      = C.hero.statusText;

  // ── About
  document.getElementById('about-pull-quote').textContent = C.about.pullQuote;

  if (C.about.headshot) {
    const img = document.getElementById('about-headshot');
    img.src = C.about.headshot;
    img.style.display = 'block';
  }

  const aboutBody = document.getElementById('about-body');
  C.about.paragraphs.forEach((p, i) => {
    const el = document.createElement('p');
    el.className = `reveal delay-${Math.min(i + 1, 4)}`;
    el.textContent = p;
    aboutBody.appendChild(el);
  });

  const currently = document.createElement('div');
  currently.className = 'about-currently reveal delay-4';
  currently.innerHTML = `<p class="currently-label">Currently</p>`;
  C.about.currently.forEach(item => {
    currently.innerHTML += `
      <div class="currently-item">
        <span class="currently-dash">—</span>
        <span class="currently-text">${item}</span>
      </div>`;
  });
  aboutBody.appendChild(currently);

  // ── Experience
  const expContainer = document.getElementById('experience-entries');
  C.experience.forEach((entry, i) => {
    const statsHtml = entry.stats.length ? `
      <div class="exp-stats">
        ${entry.stats.map(s => `
          <div class="exp-stat">
            <span class="exp-stat-num">${s.number}</span>
            <span class="exp-stat-label">${s.label}</span>
          </div>`).join('')}
      </div>` : '';

    const el = document.createElement('div');
    el.className = `exp-entry reveal delay-${Math.min(i % 4 + 1, 4)}`;
    el.innerHTML = `
      <div class="exp-meta">
        <p class="exp-company">${entry.company}</p>
        <p class="exp-date">${entry.date}</p>
      </div>
      <div class="exp-content">
        ${entry.badge ? `<span class="exp-badge">${entry.badge}</span>` : ''}
        <h3 class="exp-role">${entry.role}</h3>
        <p class="exp-location">${entry.location}</p>
        <p class="exp-desc">${entry.description}</p>
        ${statsHtml}
      </div>`;
    expContainer.appendChild(el);
  });

  // ── Projects
  const projContainer = document.getElementById('projects-container');
  C.projects.forEach(category => {
    const section = document.createElement('div');
    section.className = 'projects-category';

    const [word1, ...rest] = category.categoryLabel.split(' & ');
    section.innerHTML = `<p class="category-label">${word1} <span>&amp;</span> ${rest.join(' & ')}</p>`;

    const grid = document.createElement('div');
    grid.className = 'projects-grid';

    category.cards.forEach((card, i) => {
      const tagsHtml = card.tags.map(t => `<span class="project-tag">${t}</span>`).join('');
      const footerLeft = card.link
        ? `<a href="${card.link}" target="_blank" rel="noopener" class="project-link">${card.linkLabel} ↗</a>`
        : '';
      const footerRight = statusBadge(card.statusType, card.statusLabel);

      const el = document.createElement('div');
      el.className = `project-card reveal delay-${Math.min(i + 1, 4)}`;
      el.innerHTML = `
        <p class="project-label">${card.label}</p>
        <h3 class="project-title">${card.title}</h3>
        <div class="project-tags">${tagsHtml}</div>
        <p class="project-desc">${card.description}</p>
        <p class="project-outcome">${card.outcome}</p>
        ${card.verifiedBadge ? `<a href="${card.verifiedBadge.url}" target="_blank" rel="noopener" class="project-verified-badge">
          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M9 12l2 2 4-4"/><circle cx="12" cy="12" r="9"/></svg>
          ${card.verifiedBadge.text}
        </a>` : ''}
        <div class="project-footer">
          ${footerLeft}
          ${footerRight}
        </div>`;
      grid.appendChild(el);
    });

    section.appendChild(grid);
    projContainer.appendChild(section);
  });

  // ── Resume CTA
  // Italicise the last word of the headline
  const words = C.resumeCta.headline.split(' ');
  const last  = words.pop();
  document.getElementById('resume-headline').innerHTML = `${words.join(' ')} <em>${last}</em>`;
  document.getElementById('resume-btn-label').textContent = C.resumeCta.buttonLabel;
  document.getElementById('resume-btn').href = C.meta.resumeFile;

  // ── Contact
  const lines = C.contact.headlineLines;
  document.getElementById('contact-heading').innerHTML =
    lines.map((l, i) => i === 1 ? `<em>${l}</em>` : l).join('<br>');
  document.getElementById('contact-closing').textContent = C.contact.closingText;

  const contactLinks = document.getElementById('contact-links');
  C.contact.links.forEach(link => {
    contactLinks.innerHTML += `
      <a href="${link.href}" class="contact-link" ${link.href.startsWith('http') ? 'target="_blank" rel="noopener"' : ''}>
        <span class="contact-link-label">${link.label}</span>
        <span class="contact-link-value">${link.value}</span>
      </a>`;
  });

  // ── Footer
  document.getElementById('footer-name').textContent = `${C.meta.name} © ${C.footer.year}`;
  document.getElementById('footer-note').textContent = C.footer.note;
}

// ── Nav scroll behaviour
window.addEventListener('scroll', () => {
  document.getElementById('nav').classList.toggle('scrolled', window.scrollY > 60);
});

// ── Scroll reveal
function initReveal() {
  // If loaded inside an iframe (e.g. PoorJar dashboard), skip animation and show everything
  let inIframe = false;
  try { inIframe = window.self !== window.top; } catch(e) { inIframe = true; }
  if (inIframe) {
    // Neutralize viewport-height based sizing so sections render at natural height
    const style = document.createElement('style');
    style.textContent = '#hero { min-height: 0 !important; height: auto !important; } section { min-height: 0 !important; }';
    document.head.appendChild(style);
    document.querySelectorAll('.reveal').forEach(el => el.classList.add('visible'));
    return;
  }
  const observer = new IntersectionObserver((entries) => {
    entries.forEach(e => {
      if (e.isIntersecting) { e.target.classList.add('visible'); observer.unobserve(e.target); }
    });
  }, { threshold: 0.1, rootMargin: '0px 0px -40px 0px' });
  document.querySelectorAll('.reveal').forEach(el => observer.observe(el));
}

// ── Mobile menu
const hamburger = document.getElementById('nav-hamburger');
const navLinks = document.getElementById('nav-links');
hamburger.addEventListener('click', () => {
  hamburger.classList.toggle('open');
  navLinks.classList.toggle('open');
});
navLinks.querySelectorAll('a').forEach(link => {
  link.addEventListener('click', () => {
    hamburger.classList.remove('open');
    navLinks.classList.remove('open');
  });
});

// Render first, then observe (so dynamically created elements are in the DOM)
render();
initReveal();

// ── Secret Terminal ──
(function() {
  const secretTrigger = document.getElementById('footer-secret');
  const terminal = document.getElementById('secretTerminal');
  const terminalInput = document.getElementById('terminalInput');
  const terminalOutput = document.getElementById('terminalOutput');
  const PASSWORD = 'osint';
  
  secretTrigger.addEventListener('click', (e) => {
    e.preventDefault();
    terminal.classList.add('open');
    terminalInput.value = '';
    terminalOutput.textContent = '';
    terminalOutput.className = 'terminal-output';
    setTimeout(() => terminalInput.focus(), 100);
  });
  
  terminal.addEventListener('click', (e) => {
    if (e.target === terminal) {
      terminal.classList.remove('open');
    }
  });
  
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && terminal.classList.contains('open')) {
      terminal.classList.remove('open');
    }
  });
  
  terminalInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      const value = terminalInput.value.toLowerCase().trim();
      
      if (value === PASSWORD) {
        terminalOutput.textContent = 'ACCESS GRANTED';
        terminalOutput.className = 'terminal-output success';
        
        setTimeout(() => {
          window.location.href = '/osint/';
        }, 800);
      } else if (value === '') {
        // Do nothing on empty
      } else {
        terminalOutput.textContent = 'ACCESS DENIED';
        terminalOutput.className = 'terminal-output';
        terminalInput.value = '';
      }
    }
  });
})();

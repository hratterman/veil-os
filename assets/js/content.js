// ============================================================
// content.js — ALL editable content lives here.
// To update the site, only ever touch this file.
// Save and reload the browser. No build step needed.
// ============================================================

const CONTENT = {

// ── META ─────────────────────────────────────────────────
meta: {
name: "Henry Ratterman",
title: "Henry Ratterman — Marketer who ships.",
description: "Marketing student at Indiana University's Kelley School of Business. Ships AI products. Builds brands. Usually both.",
email: "henry@henryratterman.com",
linkedin: "https://linkedin.com/in/HenryRatterman",
resumeFile: "resume.pdf",
},

// ── HERO ─────────────────────────────────────────────────
hero: {
eyebrow: "Marketer who ships.",
firstName: "Henry",
lastName: "Ratterman",
tagline: "Marketing student who ships AI products. It makes more sense than it sounds.",
statusText: "Open to full-time opportunities · Spring 2027",
},

// ── ABOUT ────────────────────────────────────────────────
about: {
headshot: "headshot.jpg",
pullQuote: "Most marketers don't build. I do.",

paragraphs: [
"Marketing student at Indiana University's Kelley School of Business. Also a founder.",
"The marketing track: category strategy at NAPA last summer, accessories growth at Ford's Lincoln division this summer. I run programming for Union Board, IU's largest student events organization.",
"Arduous is my company. Candidates take a 30-minute AI business simulation and either earn a public, linkable credential or they don't. I built it myself. It's live at arduous.io.",
"I also run a home server out of my apartment. This site is on it. I got into self-hosting because I wanted to ship things without waiting on anyone, and that's still the reason I keep it running.",
"Spring 2027.",
],

currently: [
"Founder, Arduous — AI fluency assessment platform · arduous.io",
"Marketing Intern, Ford Motor Company · Lincoln Division · Dearborn, MI (Summer 2026)",
"Programming Director, Indiana University Union Board",
"Open to product, growth, and brand roles in Chicago or SF post-grad",
],
},

// ── EXPERIENCE ───────────────────────────────────────────
experience: [
{
company: "Ford Motor Co.",
date: "Summer 2026",
role: "Marketing Intern, Lincoln Division",
location: "Dearborn, MI · mFCG Program",
badge: "Current",
description: "Accessories growth strategy for the Lincoln brand. Building a brand POV and go-to-market roadmap for dealer-installed options.",
stats: [],
},
{
company: "NAPA Auto Parts",
date: "Summer 2025",
role: "Category Management Intern",
location: "Atlanta, GA · Genuine Parts Company",
badge: null,
description: "Found a $20M+ gap in NAPA's appearance chemicals category and built a B2B car wash program to fill it. Field research across Atlanta dealerships and detail shops, supplier negotiation, and a 3-market pilot launch strategy for national rollout.",
stats: [
{ number: "20%+", label: "Gross margin improvement" },
{ number: "3", label: "Pilot DC markets launched" },
{ number: "15%", label: "Projected 3-year category growth" },
],
},
{
company: "Waites Sensor Tech",
date: "Summer 2024",
role: "Product Management Intern",
location: "Cincinnati, OH",
badge: null,
description: "Took a new oil analysis product from concept to Fortune 500 demo in three months. Wrote user stories, coordinated with engineering, and pitched to internal and external partners.",
stats: [],
},
{
company: "IU Union Board",date: "Sep 2024 — Present",
role: "Programming Director",
location: "Bloomington, IN · Indiana University",
badge: null,
description: "Planning and running events for IU's largest student programming organization. 100+ annual events from workshops to concerts. Previously served as Marketing Director for a year and a half, building the marketing strategy from scratch, leading a 6-person committee, and establishing a cohesive brand voice across all channels.",
stats: [
{ number: "53%", label: "Engagement growth (as Marketing Director)" },
{ number: "1,000+", label: "New followers" },
{ number: "$512K", label: "Annual budget" },
],
},
],

// ── PROJECTS ─────────────────────────────────────────────
projects: [
{
categoryLabel: "Product & Building",
cards: [
{
label: "Founder · AI + Hiring",
title: "Arduous",
tags: ["Node.js", "PostgreSQL", "Claude API", "Full-Stack"],
description: "AI fluency certification. Candidates get a real business problem, 30 minutes, and any AI tools they want. What gets graded is judgment: finding the signal, catching AI mistakes, producing something a real person would act on. Pass and you get a verified profile with a written verdict.",
outcome: "Live at arduous.io. Open registration, automated grading, results in minutes.",
link: "https://arduous.io",
linkLabel: "arduous.io",
statusType: "live",
statusLabel: "Live",
},
    {
label: "Side Project · Privacy + Web",
title: "snp.gg",
tags: ["Node.js", "SQLite", "Web Crypto API", "Full-Stack"],
description: "Write something, share a link, it disappears after one read. No accounts, no logs, no analytics. Messages expire after 12 hours if unopened. The whole thing is a 4-character URL.",
outcome: "Built and shipped in an afternoon.",
link: "https://snp.gg",
linkLabel: "snp.gg",
statusType: "live",
statusLabel: "Live",
},
{
label: "Side Project · Analytics",
title: "PoorJar",
tags: ["Node.js", "Supabase", "JavaScript", "Open Source"],
description: "Free, open-source Hotjar alternative. One script tag, no account required. Tracks clicks, scroll depth, rage clicks, and session dwell time. Dashboard at poorjar.com overlays a real heatmap directly on your live site. Supports Supabase, Airtable, and Google Sheets as backends.",
outcome: "Live at poorjar.com. Open source on GitHub.",
link: "https://poorjar.com",
linkLabel: "poorjar.com",
statusType: "live",
statusLabel: "Live",
},
{
label: "Side Project · AI + Telephony",
title: "Phony.ai",
tags: ["FastAPI", "Twilio", "OpenAI Realtime API", "PostgreSQL", "Full-Stack"],
description: "Type what you need. Phony finds the business, calls it, navigates phone menus with DTMF, holds a real multi-turn conversation with a human, and comes back with a summary and recording. Built on OpenAI's Realtime API with bidirectional WebSocket audio streaming, AI-powered call screening, and live transcript tracking.",
outcome: "Handles IVR menus, holds, transfers, and multi-step conversations autonomously.",
link: "https://callphony.ai",
linkLabel: "callphony.ai",
statusType: "live",
statusLabel: "Live",
},
{
label: "Side Project · Machine Learning",
title: "Imprint",
tags: ["TensorFlow.js", "MediaPipe", "JavaScript", "In-Browser ML"],
description: "Train a real neural network with your webcam — entirely in your browser. Collect a few samples, hit Train, and the model learns to recognize whatever you showed it in seconds. Supports hand gesture tracking, face expression detection, MobileNet transfer learning, and a fast CNN. Zero data ever leaves your device.",
outcome: "Four ML modes. Runs on any laptop. No server, no API, no install.",
link: "https://henryratterman.com/imprint",
linkLabel: "henryratterman.com/imprint",
statusType: "live",
statusLabel: "Live",
},
{
label: "Client Project",
title: "Galley: AI Book Editor",
tags: ["React", "Express", "Claude API", "PostgreSQL", "InDesign IDML"],
description: "An AI-powered editorial web app for print production. Ingests Adobe InDesign IDML files, runs paragraph-level AI review with tracked-changes-style diffs, and gives editors an approve/deny/edit workflow — then exports corrected, print-ready IDML. Built for a real book project with a real deadline.",
outcome: "Replacing a manual editorial process with an AI-assisted workflow.",
link: "https://galley.henryratterman.com",
linkLabel: "galley.henryratterman.com",
statusType: "live",
statusLabel: "Live",
},
{
label: "Side Project · Transit + Maps",
title: "Transit Display",
tags: ["Node.js", "GTFS-RT", "SVG", "Real-Time Data", "Full-Stack"],
description: "Real-time transit map that runs in the browser. Every dot is a live train, pulled from the public GTFS feeds that transit agencies publish. I process each city's static feed into route geometry once, then poll the realtime feed every 12 seconds, match vehicles to shapes, and render everything as a single SVG — no mapping library, no tiles. 17 cities so far: NYC, Chicago, SF, Boston, DC, Seattle, Denver, Portland, Minneapolis, Toronto, Brisbane, and more.",
outcome: "Each city has its own quirks. NYC only publishes trip updates, not vehicle positions, so I estimate location from upcoming stop sequences.",
link: "https://transit.henryratterman.com",
linkLabel: "transit.henryratterman.com",
statusType: "live",
statusLabel: "Live",
},
{
label: "Side Project · Offline Knowledge",
title: "Apocalypse",
tags: ["Python", "libzim", "PyInstaller", "Kiwix", "LLM"],
description: "Plug in a USB drive or SD card and get Wikipedia, Stack Overflow, iFixit, medical references, homesteading guides, and a local LLM that actually answers questions using those sources. No internet. No subscription. Works on any Mac, Windows, or Linux machine without installing anything.",
outcome: "The drive is the app. Unplug and take it anywhere.",
link: "https://apocalypse.henryratterman.com",
linkLabel: "apocalypse.henryratterman.com",
statusType: "live",
statusLabel: "Live",
},
],
},
{
categoryLabel: "Brand & Marketing",
cards: [
{
label: "Category Management · NAPA Auto Parts",
title: "Commercial Wash Program",
tags: ["Category Strategy", "Supplier Negotiation", "B2B Go-to-Market", "Field Research"],
description: "Found a $20M+ gap in NAPA's appearance chemicals category. Built a commercial car wash program from zero: market sizing, supplier selection, product testing, pricing negotiation, and a 3-market pilot launch targeting dealerships, detail shops, and car washes.",
outcome: "Pilot live in Minneapolis, Kansas City, and Miami. Projected to grow the category 15% over 3 years.",
link: null,
linkLabel: null,
statusType: "live",
statusLabel: "Pilot live in 3 markets",
},
{
label: "IU Consumer Marketing Workshop · Scotts Miracle-Gro",
title: "Scan. Grow. Thrive.",
tags: ["Consumer Insights", "Go-to-Market", "IMC Planning", "Brand Strategy"],
description: "Go-to-market strategy for Bonnie Plants' personalized gardening platform, developed for Scotts Miracle-Gro's IU workshop competition. Consumer research, IMC plan, QR-to-grow-calendar product flow, ROI modeling, and implementation timeline — presented live to SMG and Bonnie Plants marketing and R&D leadership.",
outcome: "Received positive feedback from Scotts Miracle-Gro and Bonnie Plants leadership.",
link: null,
linkLabel: null,
statusType: null,
statusLabel: "Presented to client · April 2025",
},
{
label: "Indiana University · Union Board",
title: "Brand & Social Strategy",
tags: ["Social Media", "Event Marketing", "Brand Voice", "Team Leadership"],
description: "Built the marketing engine for IU's largest student programming organization — 100+ annual events, from workshops to concerts. Led a 6-person committee, established a cohesive brand voice, and coordinated across designers, organizers, and leadership.",outcome: "53% increase in engagement, 1,000+ new followers, 3,000+ ticket sales for flagship events.",
link: null,
linkLabel: null,
statusType: null,
statusLabel: "Completed",
},
{
label: "Internship · Waites Sensor Technologies",
title: "Oil Analysis Product Launch",
tags: ["Product Management", "Jira", "User Stories", "B2B", "Fortune 500"],
description: "Took a new oil analysis product from concept to Fortune 500 demo in three months. Wrote user stories, coordinated with the engineering team, and built the pitch materials for internal and external partners.",
outcome: "Product demoed to a Fortune 500 client by end of summer.",
link: null,
linkLabel: null,
statusType: null,
statusLabel: "Summer 2024",
},
],
},
],

// ── RESUME CTA ───────────────────────────────────────────
resumeCta: {
headline: "Want the full story?",
buttonLabel: "Download Resume",
},

// ── CONTACT ──────────────────────────────────────────────
contact: {
headlineLines: ["Want to", "talk?"],
closingText: "Always happy to talk about building, product, marketing, or whatever. Reach out.",
links: [
{ label: "Email", value: "henry@henryratterman.com", href: "mailto:henry@henryratterman.com" },
{ label: "LinkedIn", value: "linkedin.com/in/HenryRatterman", href: "https://linkedin.com/in/HenryRatterman" },
{ label: "Twitter", value: "@henryratterman", href: "https://x.com/henryratterman" },
{ label: "GitHub", value: "github.com/hratterman", href: "https://github.com/hratterman" },
],
},

// ── BLOG TEASER ──────────────────────────────────────────
blog: {
teaser: true,
latestPostUrl: "/blog/post.html?slug=building-phony-ai",
},

// ── FOOTER ───────────────────────────────────────────────
footer: {
year: "2026",
note: "Self-hosted on a Mac Mini. Usually in Detroit.",
},

};

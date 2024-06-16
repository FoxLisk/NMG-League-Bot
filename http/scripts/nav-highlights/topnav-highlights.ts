import { Page, ACTIVE_NAV_CLASS_NAME } from './constants.js';

const TOP_NAV_SELECTOR = '#top-nav';
const currentSeasonNumber: string = (document.querySelector(TOP_NAV_SELECTOR) as HTMLElement)?.dataset.currentSeasonOrdinal ?? '-1';

const TOPNAV_PAGES: Page[] = [
  // Home
  {
    pathFormat: new RegExp('^/$'),
    navItemSelector: '#home-link',
  },

  // Current Season
  {
    pathFormat: new RegExp(`^/season/${currentSeasonNumber}/.*$`),
    navItemSelector: '#current-season-link',
  },

  // Previous Seasons
  {
    // Match `seasons` or any season detail page
    // Current season number already covered above so all other season numbers would be previous seasons
    pathFormat: new RegExp(`^/season(s|/.*)$`),
    navItemSelector: '#previous-seasons-link',
  },

  // Asyncs
  {
    pathFormat: new RegExp('^/asyncs$'),
    navItemSelector: '#asyncs-link',
  },

  // Login
  {
    pathFormat: new RegExp('^/login$'),
    navItemSelector: '#login-link',
  },
];

// Highlight nav items
const currentPage = TOPNAV_PAGES.find(page => location.pathname.match(page.pathFormat));
if (currentPage !== undefined) {
  document.querySelector(currentPage.navItemSelector)?.classList.add(ACTIVE_NAV_CLASS_NAME);
}

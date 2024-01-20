interface Page {
  pathFormat: RegExp;
  navItemSelector: string;
}

const TOP_NAV_SELECTOR = '#top-nav';
const currentSeasonNumber: string = (document.querySelector(TOP_NAV_SELECTOR) as HTMLElement)?.dataset.currentSeasonId ?? '-1';

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

const SEASON_DETAIL_PAGE_PATH_FORMAT = new RegExp('^/season/.*$');
const SEASON_DETAIL_SUBNAV_PAGES: Page[] = [
  // Brackets
  {
    pathFormat: new RegExp('^/season/.*?/bracket.*?$'),
    navItemSelector: '#current-season-brackets-link',
  },

  // Standings
  {
    pathFormat: new RegExp('^/season/.*?/standings$'),
    navItemSelector: '#current-season-standings-link',
  },

  // Qualifiers
  {
    pathFormat: new RegExp('^/season/.*?/qualifiers$'),
    navItemSelector: '#current-season-qualifiers-link',
  },
];

const ACTIVE_NAV_CLASS_NAME = 'nav-item-active';

// Highlight nav items
(() => {
  const currentPage = TOPNAV_PAGES.find(page => location.pathname.match(page.pathFormat));
  if (currentPage === undefined) {
    return;
  }

  document.querySelector(currentPage.navItemSelector)?.classList.add(ACTIVE_NAV_CLASS_NAME);

  // Check to see if we're on the season detail page
  if (!location.pathname.match(SEASON_DETAIL_PAGE_PATH_FORMAT)) {
    return;
  }

  // Highlight appropriate sub-page nav items on season detail page
  const subPage = SEASON_DETAIL_SUBNAV_PAGES.find(subPage => location.pathname.match(subPage.pathFormat));
  if (subPage !== undefined) {
    document.querySelector(subPage.navItemSelector)?.classList.add(ACTIVE_NAV_CLASS_NAME);
  }
})();

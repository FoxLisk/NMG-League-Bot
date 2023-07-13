interface Page {
  name: string;
  pathFormat: RegExp;
  navItemSelector: string;
}

const CURRENT_SEASON_PAGE_NAME = 'currentSeason';
const TOP_LEVEL_PAGES: Page[] = [
  {
    name: 'home',
    pathFormat: new RegExp('^/$'),
    navItemSelector: '#home-link',
  },
  {
    name: CURRENT_SEASON_PAGE_NAME,
    pathFormat: new RegExp('^/season/.*$'),
    navItemSelector: '#current-season-link',
  },
  {
    name: 'previousSeasons',
    pathFormat: new RegExp('^/seasons$'),
    navItemSelector: '#previous-seasons-link',
  },
  {
    name: 'asyncs',
    pathFormat: new RegExp('^/asyncs$'),
    navItemSelector: '#asyncs-link',
  },
  {
    name: 'login',
    pathFormat: new RegExp('^/login$'),
    navItemSelector: '#login-link',
  },
];
const CURRENT_SEASON_SUB_PAGES: Page[] = [
  {
    name: 'brackets',
    pathFormat: new RegExp('^/season/.*?/bracket.*?$'),
    navItemSelector: '#current-season-brackets-link',
  },
  {
    name: 'standings',
    pathFormat: new RegExp('^/season/.*?/standings$'),
    navItemSelector: '#current-season-standings-link',
  },
  {
    name: 'qualifiers',
    pathFormat: new RegExp('^/season/.*?/qualifiers$'),
    navItemSelector: '#current-season-qualifiers-link',
  },
];

const ACTIVE_NAV_CLASS_NAME = 'nav-item-active';

// Highlight nav items
(() => {
  const currentPage = TOP_LEVEL_PAGES.find(page => location.pathname.match(page.pathFormat));
  if (currentPage === undefined) {
    return;
  }
  document.querySelector(currentPage.navItemSelector)?.classList.add(ACTIVE_NAV_CLASS_NAME);

  if (currentPage.name !== CURRENT_SEASON_PAGE_NAME) {
    return;
  }

  // On a `current-season` page. Highlight appropriate sub-page nav item.
  const subPage = CURRENT_SEASON_SUB_PAGES.find(subPage => location.pathname.match(subPage.pathFormat));
  if (subPage !== undefined) {
    document.querySelector(subPage.navItemSelector)?.classList.add(ACTIVE_NAV_CLASS_NAME);
  }
})();

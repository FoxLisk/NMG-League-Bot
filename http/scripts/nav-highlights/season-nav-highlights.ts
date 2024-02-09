import { Page, ACTIVE_NAV_CLASS_NAME } from './constants.js';

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

const subPage = SEASON_DETAIL_SUBNAV_PAGES.find(subPage => location.pathname.match(subPage.pathFormat));
if (subPage !== undefined) {
  document.querySelector(subPage.navItemSelector)?.classList.add(ACTIVE_NAV_CLASS_NAME);
}

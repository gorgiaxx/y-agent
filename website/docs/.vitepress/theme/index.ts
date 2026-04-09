import DefaultTheme from 'vitepress/theme';
import HomeLayout from './HomeLayout.vue';
import DownloadHero from './DownloadHero.vue';
import './custom.css';

export default {
  extends: DefaultTheme,
  Layout: HomeLayout,
  enhanceApp({ app }) {
    app.component('DownloadHero', DownloadHero);
  },
};

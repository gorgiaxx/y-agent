<script setup lang="ts">
import DefaultTheme from 'vitepress/theme';
import DownloadHero from './DownloadHero.vue';
import GuiScreenshot from './GuiScreenshot.vue';

const { Layout } = DefaultTheme;
</script>

<template>
  <Layout>
    <template #home-hero-after>
      <GuiScreenshot />
    </template>
    <template #home-features-before>
      <div class="home-download-section">
        <DownloadHero />
      </div>
    </template>
  </Layout>
</template>

<style scoped>
.home-download-section {
  max-width: 1152px;
  margin: 0 auto;
  padding: 0 24px;
}

@media (min-width: 640px) {
  .home-download-section {
    padding: 0 48px;
  }
}

@media (min-width: 960px) {
  .home-download-section {
    padding: 0 64px;
  }
}
</style>

<script setup lang="ts">
</script>

<template>
  <div class="gui-screenshot">
    <div class="screenshot-container">
      <img
        src="/gui_light.png"
        alt="y-agent GUI - Light theme"
        class="screenshot-img light-img"
        loading="lazy"
      />
      <img
        src="/gui_dark.png"
        alt="y-agent GUI - Dark theme"
        class="screenshot-img dark-img"
        loading="lazy"
      />
    </div>
  </div>
</template>

<style scoped>
.gui-screenshot {
  display: flex;
  justify-content: center;
  padding: 0 24px;
  margin-top: -24px;
}

.screenshot-container {
  position: relative;
  width: 100%;
  max-width: 960px;
}

.screenshot-img {
  display: block;
  width: 100%;
  height: auto;
}

/* Light mode: show light screenshot, hide dark */
.light-img {
  display: block;
}
.dark-img {
  display: none;
}

/* Dark mode: show dark screenshot, hide light */
:root:not(.dark) .light-img {
  display: block;
}
:root:not(.dark) .dark-img {
  display: none;
}
.dark .light-img {
  display: none;
}
.dark .dark-img {
  display: block;
}

@media (max-width: 640px) {
  .gui-screenshot {
    padding: 0 12px;
    margin-top: -16px;
  }
}
</style>

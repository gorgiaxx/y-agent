<script setup lang="ts">
import { ref, computed, onMounted } from 'vue';

const REPO = 'https://github.com/gorgias/y-agent';

interface DownloadItem {
  label: string;
  desc: string;
  os: 'macos' | 'windows' | 'linux';
}

const downloads: DownloadItem[] = [
  { os: 'macos', label: 'macOS (Apple Silicon)', desc: '.dmg for M-series chips' },
  { os: 'macos', label: 'macOS (Intel)', desc: '.dmg for Intel Macs' },
  { os: 'windows', label: 'Windows (x64)', desc: '.msi installer' },
  { os: 'linux', label: 'Linux (x64 .deb)', desc: 'Debian / Ubuntu' },
  { os: 'linux', label: 'Linux (x64 AppImage)', desc: 'Universal Linux' },
];

const detectedOS = ref<'macos' | 'windows' | 'linux'>('macos');
const showAll = ref(false);

onMounted(() => {
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes('mac')) {
    detectedOS.value = 'macos';
  } else if (ua.includes('win')) {
    detectedOS.value = 'windows';
  } else if (ua.includes('linux')) {
    detectedOS.value = 'linux';
  }
});

const recommendedItems = computed<DownloadItem[]>(() => {
  return downloads.filter((d) => d.os === detectedOS.value);
});

const otherDownloads = computed(() => {
  const recSet = new Set(recommendedItems.value);
  return downloads.filter((d) => !recSet.has(d));
});
</script>

<template>
  <div class="download-hero">
    <div class="primary-download">
      <a
        :href="`${REPO}/releases`"
        class="download-btn primary"
        target="_blank"
        rel="noopener"
      >
        <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
        Download from GitHub Releases
      </a>
    </div>

    <button class="toggle-btn" @click="showAll = !showAll">
      {{ showAll ? 'Collapse' : 'View All Platforms' }}
      <svg :class="{ rotated: showAll }" xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>
    </button>

    <Transition name="slide">
      <div v-if="showAll" class="all-downloads">
        <template v-for="group in ['macos', 'windows', 'linux']" :key="group">
          <div class="os-group">
            <h4 class="os-title">
              {{ group === 'macos' ? 'macOS' : group === 'windows' ? 'Windows' : 'Linux' }}
            </h4>
            <div class="download-grid">
              <div
                v-for="item in downloads.filter((d) => d.os === group)"
                :key="item.label"
                class="download-card"
              >
                <span class="card-label">{{ item.label }}</span>
                <span class="card-desc">{{ item.desc }}</span>
              </div>
            </div>
          </div>
        </template>
        <p class="releases-link">
          All binaries are available on the
          <a :href="`${REPO}/releases`" target="_blank" rel="noopener">GitHub Releases</a> page.
        </p>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.download-hero {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
  margin: 32px 0;
}

.primary-download {
  display: flex;
  align-items: center;
  justify-content: center;
  flex-wrap: wrap;
  gap: 12px;
}

.download-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  border-radius: 20px;
  font-weight: 500;
  text-decoration: none;
  transition: border-color 0.25s, color 0.25s, background-color 0.25s;
  cursor: pointer;
  white-space: nowrap;
}

.download-btn.primary {
  padding: 0 24px;
  height: 48px;
  font-size: 16px;
  line-height: 48px;
  color: #fff;
  background-color: var(--vp-c-brand-1);
  border: 2px solid transparent;
}
.download-btn.primary:hover {
  background-color: var(--vp-c-brand-2);
}

.toggle-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  background: none;
  border: none;
  color: var(--vp-c-brand-1);
  cursor: pointer;
  font-size: 14px;
  padding: 4px 8px;
}
.toggle-btn:hover {
  text-decoration: underline;
}
.toggle-btn svg {
  transition: transform 0.2s;
}
.toggle-btn svg.rotated {
  transform: rotate(180deg);
}

.all-downloads {
  width: 100%;
  max-width: 600px;
}

.os-group {
  margin-bottom: 16px;
}

.os-title {
  font-size: 14px;
  font-weight: 600;
  margin-bottom: 8px;
  color: var(--vp-c-text-2);
}

.download-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.download-card {
  display: flex;
  flex-direction: column;
  padding: 10px 16px;
  border: 1px solid var(--vp-c-divider);
  border-radius: 8px;
  background-color: var(--vp-c-bg-soft);
}

.card-label {
  font-size: 14px;
  font-weight: 500;
  color: var(--vp-c-text-1);
}

.card-desc {
  font-size: 12px;
  color: var(--vp-c-text-3);
}

.releases-link {
  font-size: 13px;
  color: var(--vp-c-text-3);
  text-align: center;
  margin-top: 8px;
}
.releases-link a {
  color: var(--vp-c-brand-1);
}

.slide-enter-active,
.slide-leave-active {
  transition: all 0.25s ease;
}
.slide-enter-from,
.slide-leave-to {
  opacity: 0;
  transform: translateY(-8px);
}
</style>

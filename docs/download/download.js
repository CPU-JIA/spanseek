const root = document.documentElement;
const themeButton = document.querySelector('.theme-toggle');
const themedImages = [...document.querySelectorAll('img[data-dark-src]')];
const THEME_KEY = 'spanseek-theme-v2';

function currentTheme() {
  return root.dataset.theme === 'dark' ? 'dark' : 'light';
}

function applyTheme(theme) {
  root.dataset.theme = theme;
  for (const image of themedImages) {
    const lightSrc = image.dataset.lightSrc || image.getAttribute('src');
    if (!lightSrc || !image.dataset.darkSrc) continue;
    image.dataset.lightSrc = lightSrc;
    const nextSrc = theme === 'dark' ? image.dataset.darkSrc : lightSrc;
    if (image.getAttribute('src') !== nextSrc) image.setAttribute('src', nextSrc);
  }
  if (themeButton) {
    const dark = theme === 'dark';
    themeButton.setAttribute('aria-pressed', String(dark));
    themeButton.setAttribute('aria-label', dark ? '切换浅色模式' : '切换深色模式');
  }
}

function suppressThemeTransitions() {
  root.classList.add('theme-switching');
  void document.body.offsetWidth;
  window.requestAnimationFrame(() => {
    window.requestAnimationFrame(() => root.classList.remove('theme-switching'));
  });
}

applyTheme(currentTheme());

themeButton?.addEventListener('click', () => {
  const next = currentTheme() === 'dark' ? 'light' : 'dark';
  suppressThemeTransitions();
  applyTheme(next);
  try {
    localStorage.setItem(THEME_KEY, next);
  } catch {}
});

for (const group of document.querySelectorAll('.preview-switcher')) {
  const buttons = [...group.querySelectorAll('[data-preview]')];
  const activate = (button) => {
    const image = document.getElementById(button.dataset.preview);
    if (!image) return;
    image.dataset.lightSrc = button.dataset.light;
    image.dataset.darkSrc = button.dataset.dark;
    image.alt = button.dataset.alt;
    for (const item of buttons) {
      item.setAttribute('aria-selected', String(item === button));
      item.tabIndex = item === button ? 0 : -1;
    }
    image.closest('[role="tabpanel"]')?.setAttribute('aria-labelledby', button.id);
    applyTheme(currentTheme());
  };
  for (const button of buttons) button.addEventListener('click', () => activate(button));
  group.addEventListener('keydown', (event) => {
    const index = buttons.indexOf(document.activeElement);
    if (index < 0) return;
    const next =
      event.key === 'ArrowRight'
        ? (index + 1) % buttons.length
        : event.key === 'ArrowLeft'
          ? (index + buttons.length - 1) % buttons.length
          : event.key === 'Home'
            ? 0
            : event.key === 'End'
              ? buttons.length - 1
              : null;
    if (next !== null) {
      event.preventDefault();
      activate(buttons[next]);
      buttons[next].focus();
    }
  });
}

const motionVideo = document.querySelector('[data-motion-video]');
const motionLaunch = document.querySelector('[data-motion-launch]');
const motionFrame = document.querySelector('[data-motion-frame]');
const motionLabel = document.querySelector('[data-motion-label]');
const motionToggle = document.querySelector('[data-motion-toggle]');
const motionMute = document.querySelector('[data-motion-mute]');
const motionCC = document.querySelector('[data-motion-cc]');
const motionFeedback = document.querySelector('[data-motion-feedback]');
let captionsEnabled = true;
let motionFailed = false;
let motionAttempt = 0;
const failedSources = new Set();

function setMotionState() {
  const playing = Boolean(
    motionVideo && !motionFailed && !motionVideo.paused && !motionVideo.ended,
  );
  const started = Boolean(
    motionVideo?.currentTime > 0 && !motionVideo.ended && !motionFailed && !motionVideo.error,
  );
  motionFrame?.classList.toggle('is-playing', playing);
  motionFrame?.classList.toggle('is-started', started);
  const label =
    motionFailed || motionVideo?.error
      ? '重试播放'
      : playing
        ? '暂停演示'
        : motionVideo?.ended
          ? '重新观看'
          : motionVideo?.currentTime > 0
            ? '继续观看'
            : '播放演示';
  if (motionLabel) motionLabel.textContent = label;
  if (motionToggle) motionToggle.textContent = label;
  motionLaunch?.setAttribute('aria-label', `${label}：50 秒跨索产品电影`);
  if (motionLaunch) {
    motionLaunch.tabIndex = playing || started ? -1 : 0;
    motionLaunch.setAttribute('aria-hidden', String(playing || started));
    if (playing && document.activeElement === motionLaunch)
      motionVideo.focus({ preventScroll: true });
  }
  if (motionMute) {
    motionMute.textContent = motionVideo?.muted ? '声音：关' : '声音：开';
    motionMute.setAttribute('aria-pressed', String(Boolean(motionVideo?.muted)));
  }
}
function setCaptions() {
  if (!motionVideo) return;
  for (const track of motionVideo.textTracks) track.mode = captionsEnabled ? 'showing' : 'disabled';
  if (motionCC) {
    motionCC.textContent = captionsEnabled ? '字幕：开' : '字幕：关';
    motionCC.setAttribute('aria-pressed', String(captionsEnabled));
  }
  positionCaptions();
}
function positionCaptions() {
  if (!motionVideo?.videoWidth || !motionVideo.clientWidth || !motionVideo.clientHeight) return;
  const width = motionVideo.clientWidth;
  const height = motionVideo.clientHeight;
  const ratio = motionVideo.videoWidth / motionVideo.videoHeight;
  const pictureWidth = Math.min(width, height * ratio);
  const pictureHeight = Math.min(height, width / ratio);
  const pictureTop = (height - pictureHeight) / 2;
  const fontSize = innerWidth <= 720 ? 14 : Math.min(24, Math.max(14, innerWidth * 0.0155));
  const measure = document.createElement('canvas').getContext('2d');
  if (!measure) return;
  measure.font = `500 ${fontSize}px ${getComputedStyle(motionVideo).fontFamily}`;
  const usableWidth = pictureWidth * 0.88;
  // Keep the caption in the film's footer, just above its gold rule.
  const bottom = pictureTop + pictureHeight * 0.955;
  for (const track of motionVideo.textTracks) {
    for (const cue of track.cues || []) {
      const lines = Math.max(1, Math.ceil(measure.measureText(cue.text).width / usableWidth));
      const top = Math.max(pictureTop, bottom - lines * fontSize * 1.45);
      cue.snapToLines = false;
      cue.line = (top / height) * 100;
      cue.position = 50;
      cue.size = (usableWidth / width) * 100;
      cue.align = 'center';
    }
  }
}
async function playMotion() {
  if (!motionVideo) return;
  if (!motionFailed && !motionVideo.paused && !motionVideo.ended) {
    motionVideo.pause();
    return;
  }
  if (motionFeedback) motionFeedback.textContent = '';
  if (motionToggle) motionToggle.disabled = true;
  const attempt = ++motionAttempt;
  try {
    const reload = motionFailed || motionVideo.error;
    motionFailed = false;
    if (reload) {
      failedSources.clear();
      motionVideo.load();
    }
    if (motionVideo.ended) motionVideo.currentTime = 0;
    await motionVideo.play();
  } catch (failure) {
    if (
      attempt === motionAttempt &&
      (failure?.name !== 'AbortError' || motionFailed) &&
      motionFeedback
    )
      motionFeedback.textContent = '暂时无法播放，请重试或下载影片观看。';
  } finally {
    if (attempt === motionAttempt) {
      if (motionToggle) motionToggle.disabled = false;
      setMotionState();
    }
  }
}
if (motionVideo) {
  motionLaunch?.addEventListener('click', playMotion);
  motionToggle?.addEventListener('click', playMotion);
  motionMute?.addEventListener('click', () => {
    motionVideo.muted = !motionVideo.muted;
  });
  motionCC?.addEventListener('click', () => {
    captionsEnabled = !captionsEnabled;
    setCaptions();
  });
  for (const name of ['play', 'pause', 'ended', 'volumechange', 'seeked'])
    motionVideo.addEventListener(name, setMotionState);
  motionVideo.addEventListener('loadedmetadata', setCaptions);
  for (const track of motionVideo.querySelectorAll('track'))
    track.addEventListener('load', positionCaptions);
  new ResizeObserver(positionCaptions).observe(motionVideo);
  document.addEventListener('fullscreenchange', positionCaptions);
  window.addEventListener('resize', positionCaptions);
  const failed = () => {
    motionFailed = true;
    motionVideo.pause();
    if (motionFeedback) motionFeedback.textContent = '暂时无法播放，请重试或下载影片观看。';
    if (motionToggle) motionToggle.disabled = false;
    setMotionState();
  };
  motionVideo.addEventListener('error', failed);
  const sources = [...motionVideo.querySelectorAll('source')];
  for (const source of sources)
    source.addEventListener('error', () => {
      failedSources.add(source);
      if (failedSources.size === sources.length) failed();
    });
  motionVideo.textTracks.addEventListener('change', () => {
    captionsEnabled = [...motionVideo.textTracks].some((track) => track.mode === 'showing');
    positionCaptions();
    if (motionCC) {
      motionCC.textContent = captionsEnabled ? '字幕：开' : '字幕：关';
      motionCC.setAttribute('aria-pressed', String(captionsEnabled));
    }
  });
  if ('IntersectionObserver' in window)
    new IntersectionObserver((entries) => {
      if (!entries[0].isIntersecting && !motionVideo.paused) motionVideo.pause();
    }).observe(motionVideo);
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) motionVideo.pause();
  });
}

async function copyText(text) {
  if (navigator.clipboard && window.isSecureContext) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const area = document.createElement('textarea');
  area.value = text;
  area.setAttribute('readonly', '');
  area.style.position = 'fixed';
  area.style.opacity = '0';
  document.body.appendChild(area);
  area.select();
  const copied = document.execCommand('copy');
  area.remove();
  if (!copied) throw new Error('copy failed');
}

for (const button of document.querySelectorAll('[data-copy]')) {
  const targetId = button.getAttribute('data-copy');
  const target = targetId ? document.getElementById(targetId) : null;
  const status = targetId ? document.querySelector(`[data-copy-status="${targetId}"]`) : null;
  if (!target || !status) continue;
  const value = target.textContent?.trim() ?? '';
  if (!value || value.startsWith('__')) {
    button.disabled = true;
    button.title = '发布信息尚未生成';
    continue;
  }
  button.addEventListener('click', async () => {
    const original = button.textContent;
    try {
      await copyText(value);
      status.textContent = '已复制到剪贴板';
      button.textContent = '已复制';
      window.setTimeout(() => {
        button.textContent = original;
        status.textContent = '';
      }, 2200);
    } catch {
      status.textContent = '复制失败，请手动选择哈希';
    }
  });
}

const spiedLinks = [
  ...document.querySelectorAll('[data-spy] a[href^="#"], .site-nav a[href^="#"]'),
];
const observed = [
  ...new Set(
    spiedLinks.map((link) => document.querySelector(link.getAttribute('href'))).filter(Boolean),
  ),
];

if ('IntersectionObserver' in window && observed.length) {
  const observer = new IntersectionObserver(
    (entries) => {
      const visible = entries
        .filter((entry) => entry.isIntersecting)
        .sort((a, b) => b.intersectionRatio - a.intersectionRatio)[0];
      if (!visible) return;
      for (const link of spiedLinks) {
        const active = link.getAttribute('href') === `#${visible.target.id}`;
        if (active) link.setAttribute('aria-current', 'location');
        else link.removeAttribute('aria-current');
      }
    },
    { rootMargin: '-18% 0px -66% 0px', threshold: [0, 0.15, 0.5] },
  );
  for (const section of observed) observer.observe(section);
}

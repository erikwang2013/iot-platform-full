const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => Array.from(root.querySelectorAll(selector));
const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;
const sections = $$('.section');
const railLinks = $$('.rail a');
const header = $('#header');
const toTop = $('#toTop');

function markPerf(name, start) {
  const ms = Math.round((performance.now() - start) * 100) / 100;
  window.__perf = window.__perf || {};
  window.__perf[name] = ms;
}

/* header state + scroll progress ring */
let scrollFrame = 0;
function syncScrollUi() {
  scrollFrame = 0;
  const y = window.scrollY;
  header.classList.toggle('is-scrolled', y > 24);
  const max = document.documentElement.scrollHeight - window.innerHeight;
  toTop.style.setProperty('--progress', max > 0 ? (y / max) * 100 : 0);
  toTop.classList.toggle('is-visible', y > window.innerHeight * 0.55);
}
addEventListener('scroll', () => {
  if (!scrollFrame) scrollFrame = requestAnimationFrame(syncScrollUi);
}, { passive: true });
syncScrollUi();

/* active section rail */
if ('IntersectionObserver' in window) {
  const railObserver = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const href = '#' + entry.target.id;
      railLinks.forEach((link) => link.classList.toggle('is-active', link.getAttribute('href') === href));
    }
  }, { rootMargin: '-45% 0px -45% 0px', threshold: 0 });
  sections.forEach((section) => railObserver.observe(section));
}

/* reveal on scroll */
const revealItems = $$('[data-reveal]');
if (reducedMotion) {
  revealItems.forEach((el) => el.classList.add('in'));
} else if ('IntersectionObserver' in window) {
  const revealObserver = new IntersectionObserver((entries, observer) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      entry.target.classList.add('in');
      observer.unobserve(entry.target);
    }
  }, { threshold: 0.12 });
  revealItems.forEach((el) => revealObserver.observe(el));
} else {
  revealItems.forEach((el) => el.classList.add('in'));
}

/* pet expression switcher */
const petGrid = $('#petGrid');
const petPreview = $('#petPreview');
const petPreviewImg = $('#petPreviewImg');
const petPreviewLabel = $('#petPreviewLabel');
let activeCell = petGrid ? petGrid.querySelector('.pet-cell.is-active') : null;
if (petGrid) {
  petGrid.addEventListener('click', (event) => {
    const cell = event.target.closest('.pet-cell');
    if (!cell || cell === activeCell) return;
    const start = performance.now();
    if (activeCell) activeCell.classList.remove('is-active');
    cell.classList.add('is-active');
    activeCell = cell;
    petPreviewImg.src = cell.dataset.src;
    petPreviewImg.alt = cell.dataset.name + ' 表情';
    petPreviewLabel.textContent = cell.dataset.label;
    markPerf('petSwitch', start);
  });
}

/* back to top */
if (toTop) {
  toTop.addEventListener('click', () => {
    const start = performance.now();
    window.scrollTo({ top: 0, behavior: reducedMotion ? 'auto' : 'smooth' });
    markPerf('backToTop', start);
  });
}

/* nav click latency marker */
$$('a[href^="#"]').forEach((link) => {
  link.addEventListener('click', () => {
    const start = performance.now();
    markPerf('navClick', start);
  });
});

/* keyboard section navigation */
addEventListener('keydown', (event) => {
  if (event.altKey || event.ctrlKey || event.metaKey) return;
  const forward = event.key === 'ArrowDown' || event.key === 'PageDown';
  const backward = event.key === 'ArrowUp' || event.key === 'PageUp';
  if (!forward && !backward) return;
  event.preventDefault();
  const start = performance.now();
  const probe = window.scrollY + window.innerHeight * 0.5;
  let index = sections.findIndex((section) => section.offsetTop <= probe && section.offsetTop + section.offsetHeight > probe);
  if (index < 0) index = 0;
  const nextIndex = Math.max(0, Math.min(sections.length - 1, index + (forward ? 1 : -1)));
  sections[nextIndex].scrollIntoView({ behavior: reducedMotion ? 'auto' : 'smooth', block: 'start' });
  markPerf('keyboardNav', start);
});

/* image lightbox with zoom */
const lightbox = $('#lightbox');
const lightboxStage = $('#lightboxStage');
const lightboxImg = $('#lightboxImg');
const lightboxClose = $('#lightboxClose');
const lightboxOriginal = $('#lightboxOriginal');
const zoomIn = $('#zoomIn');
const zoomOut = $('#zoomOut');
let lightboxReturn = null;
let lightboxZoom = 1;
let lightboxBaseWidth = 900;
function applyLightboxZoom() {
  lightboxImg.style.width = Math.max(120, Math.round(lightboxBaseWidth * lightboxZoom)) + 'px';
  lightboxImg.style.height = 'auto';
  if (lightboxStage) lightboxStage.style.textAlign = lightboxZoom > 1 ? 'left' : 'center';
}
function closeLightbox() {
  if (!lightbox || lightbox.hidden) return;
  lightbox.classList.remove('is-open');
  document.body.classList.remove('is-lightbox-open');
  setTimeout(() => {
    lightbox.hidden = true;
    lightboxImg.removeAttribute('src');
  }, 180);
  if (lightboxReturn && lightboxReturn.focus) lightboxReturn.focus();
}
function openLightbox(img) {
  if (!lightbox) return;
  lightboxReturn = document.activeElement;
  const source = img.currentSrc || img.src;
  const natural = img.naturalWidth || 900;
  lightboxBaseWidth = Math.max(320, Math.min(natural, window.innerWidth * 0.92));
  lightboxZoom = 1;
  lightboxImg.src = source;
  lightboxImg.alt = img.alt || '图片预览';
  applyLightboxZoom();
  if (lightboxOriginal) lightboxOriginal.href = source;
  lightbox.hidden = false;
  if (lightboxStage) { lightboxStage.scrollTop = 0; lightboxStage.scrollLeft = 0; }
  document.body.classList.add('is-lightbox-open');
  requestAnimationFrame(() => lightbox.classList.add('is-open'));
  if (lightboxClose) lightboxClose.focus();
}
if (lightbox) {
  $$('main img').forEach((img) => {
    if (img.closest('button') || img.closest('a')) return;
    img.tabIndex = 0;
    img.setAttribute('role', 'button');
    img.setAttribute('aria-label', '放大查看：' + (img.alt || '图片'));
    img.addEventListener('click', () => openLightbox(img));
    img.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        openLightbox(img);
      }
    });
  });
  if (lightboxClose) lightboxClose.addEventListener('click', closeLightbox);
  if (lightboxStage) lightboxStage.addEventListener('click', (event) => {
    if (event.target === lightboxStage) closeLightbox();
  });
  if (zoomIn) zoomIn.addEventListener('click', () => {
    lightboxZoom = Math.min(4, lightboxZoom + 0.25);
    applyLightboxZoom();
  });
  if (zoomOut) zoomOut.addEventListener('click', () => {
    lightboxZoom = Math.max(0.5, lightboxZoom - 0.25);
    applyLightboxZoom();
  });
  addEventListener('keydown', (event) => {
    if (event.key === 'Escape') closeLightbox();
  });
}

/* three.js background — loaded after first paint so interactions stay instant */
function startScene() {
  if (reducedMotion || !$('#bg')) return;
  import('./three.module.min.js').then((THREE) => { try { initScene(THREE); } catch { /* WebGL unavailable */ } }).catch(() => {});
}

function initScene(THREE) {
  const canvas = $('#bg');
  const renderer = new THREE.WebGLRenderer({ canvas, alpha: true, antialias: false, powerPreference: 'high-performance' });
  const isSmall = window.innerWidth < 820;
  const isLight = document.documentElement.dataset.theme === 'light';
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, isSmall ? 1.4 : 1.75));
  renderer.setSize(window.innerWidth, window.innerHeight);
  renderer.setClearColor(0x000000, 0);

  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(58, window.innerWidth / window.innerHeight, 0.1, 100);
  camera.position.set(0, 0, 7.2);

  const css = getComputedStyle(document.documentElement);
  const palette = ['--primary', '--secondary', '--accent'].map((name) => new THREE.Color(css.getPropertyValue(name).trim() || '#2d8cf0'));
  if (isLight) palette.forEach((color) => color.multiplyScalar(0.72));

  const count = isSmall ? 380 : 900;
  const positions = new Float32Array(count * 3);
  const colors = new Float32Array(count * 3);
  const tint = new THREE.Color();
  for (let i = 0; i < count; i += 1) {
    const radius = 3.2 + Math.random() * 6.4;
    const theta = Math.random() * Math.PI * 2;
    const phi = Math.acos(Math.random() * 2 - 1);
    positions[i * 3] = radius * Math.sin(phi) * Math.cos(theta);
    positions[i * 3 + 1] = radius * Math.sin(phi) * Math.sin(theta) * 0.7;
    positions[i * 3 + 2] = radius * Math.cos(phi);
    tint.copy(palette[i % 2]).lerp(palette[2], Math.random() * 0.35);
    colors[i * 3] = tint.r;
    colors[i * 3 + 1] = tint.g;
    colors[i * 3 + 2] = tint.b;
  }
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  geometry.setAttribute('color', new THREE.BufferAttribute(colors, 3));
  const spriteCanvas = document.createElement('canvas');
  spriteCanvas.width = 64;
  spriteCanvas.height = 64;
  const spriteCtx = spriteCanvas.getContext('2d');
  const spriteGradient = spriteCtx.createRadialGradient(32, 32, 0, 32, 32, 32);
  spriteGradient.addColorStop(0, 'rgba(255,255,255,1)');
  spriteGradient.addColorStop(0.35, 'rgba(255,255,255,.85)');
  spriteGradient.addColorStop(1, 'rgba(255,255,255,0)');
  spriteCtx.fillStyle = spriteGradient;
  spriteCtx.fillRect(0, 0, 64, 64);
  const sprite = new THREE.CanvasTexture(spriteCanvas);
  const material = new THREE.PointsMaterial({
    size: isSmall ? 0.075 : 0.06,
    map: sprite,
    alphaTest: 0.02,
    vertexColors: true,
    transparent: true,
    opacity: isLight ? 0.5 : 0.78,
    depthWrite: false,
    blending: isLight ? THREE.NormalBlending : THREE.AdditiveBlending,
    sizeAttenuation: true,
  });
  const points = new THREE.Points(geometry, material);
  scene.add(points);

  const ico = new THREE.Mesh(
    new THREE.IcosahedronGeometry(1.75, 1),
    new THREE.MeshBasicMaterial({ color: palette[0], wireframe: true, transparent: true, opacity: isLight ? 0.1 : 0.08 }),
  );
  ico.position.set(2.7, 0.3, -2.5);
  scene.add(ico);

  const pointer = { x: 0, y: 0, tx: 0, ty: 0 };
  addEventListener('pointermove', (event) => {
    pointer.tx = (event.clientX / window.innerWidth) * 2 - 1;
    pointer.ty = (event.clientY / window.innerHeight) * 2 - 1;
  }, { passive: true });

  const sectionColors = [palette[0], palette[1], palette[2], palette[0], palette[1], palette[2]];
  let targets = sections.map((section) => section.offsetTop);
  let activeIndex = 0;
  let targetColor = palette[0].clone();
  const currentColor = palette[0].clone();
  function syncColor() {
    const probe = window.scrollY + window.innerHeight * 0.35;
    let index = 0;
    for (let i = 0; i < targets.length; i += 1) if (targets[i] <= probe) index = i;
    if (index !== activeIndex) {
      activeIndex = index;
      targetColor = sectionColors[index % sectionColors.length];
    }
  }
  addEventListener('scroll', syncColor, { passive: true });
  syncColor();

  const clock = new THREE.Clock();
  let raf = 0;
  function frame() {
    const t = clock.getElapsedTime();
    pointer.x += (pointer.tx - pointer.x) * 0.04;
    pointer.y += (pointer.ty - pointer.y) * 0.04;
    points.rotation.y = t * 0.018 + pointer.x * 0.08;
    points.rotation.x = Math.sin(t * 0.12) * 0.05 + pointer.y * 0.05;
    ico.rotation.x = t * 0.14;
    ico.rotation.y = t * 0.18 + pointer.x * 0.2;
    currentColor.lerp(targetColor, 0.03);
    ico.material.color.copy(currentColor);
    material.opacity = (isLight ? 0.5 : 0.78) + Math.sin(t * 0.8) * 0.04;
    camera.position.x = pointer.x * 0.35;
    camera.position.y = -pointer.y * 0.25;
    camera.lookAt(0, 0, 0);
    renderer.render(scene, camera);
    raf = requestAnimationFrame(frame);
  }

  document.addEventListener('visibilitychange', () => {
    if (document.hidden) {
      cancelAnimationFrame(raf);
    } else {
      clock.getDelta();
      raf = requestAnimationFrame(frame);
    }
  });
  addEventListener('resize', () => {
    camera.aspect = window.innerWidth / window.innerHeight;
    camera.updateProjectionMatrix();
    renderer.setSize(window.innerWidth, window.innerHeight);
    targets = sections.map((section) => section.offsetTop);
  }, { passive: true });
  raf = requestAnimationFrame(frame);
}

if ('requestIdleCallback' in window) requestIdleCallback(startScene, { timeout: 1600 });
else setTimeout(startScene, 300);

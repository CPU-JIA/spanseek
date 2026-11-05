import { useEffect, useState } from 'react';

/** Fullscreen a work surface, keeping its controls and content together. */
export function useContentFullscreen(selector: string) {
  const [active, setActive] = useState(false);
  useEffect(() => {
    const update = () => setActive(!!document.fullscreenElement?.matches(selector));
    update();
    document.addEventListener('fullscreenchange', update);
    return () => document.removeEventListener('fullscreenchange', update);
  }, [selector]);
  async function toggle() {
    if (document.fullscreenElement?.matches(selector)) await document.exitFullscreen();
    else {
      const element = document.querySelector<HTMLElement>(selector);
      if (element) await element.requestFullscreen();
    }
  }
  return { active, toggle };
}

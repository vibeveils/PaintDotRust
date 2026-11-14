// Layout modes for different screens, and the phone menu drawer.
//
// - desktop: floating windows, menu bar, mouse-sized controls.
// - tablet:  the same layout with touch-sized controls (primary pointer is coarse).
// - phone:   narrow screens; windows become bottom (or side, in landscape)
//            sheets opened from a navigation bar, and the menus a drawer.

/** Current layout: 'phone', 'tablet' or 'desktop'. `uiScale` is the interface size (1 = 100%). */
export function detectLayout(uiScale = 1) {
  const w = innerWidth / uiScale, h = innerHeight / uiScale;
  const coarse = matchMedia('(pointer: coarse)').matches;
  // Phone-sized touch screens stay phones in landscape, whatever the
  // on-screen keyboard does to the window height.
  const device = Math.min(screen.width, screen.height);
  if (w < 640 || (coarse && device < 520) || h < 360) return 'phone';
  return coarse ? 'tablet' : 'desktop';
}

export function isTouchPrimary() {
  return matchMedia('(pointer: coarse)').matches;
}

/**
 * Opens the menus as a drawer from the left (phones): each top-level menu and
 * submenu is a collapsible section. `menus` is the menu bar's own spec
 * ({ label, items: [{ label, icon, key, enabled, checked, submenu, action } | '-'] }).
 */
export function openMenuDrawer(menus, { el, icon, title = 'Menu' }) {
  document.querySelector('.drawer-backdrop')?.remove();
  const backdrop = el('div', { class: 'drawer-backdrop' });
  const drawer = el('nav', { class: 'drawer', role: 'dialog', 'aria-modal': 'true', 'aria-label': title });
  const close = () => {
    backdrop.remove();
    document.removeEventListener('keydown', onKey, true);
    document.getElementById('menu-button')?.focus();
  };
  const onKey = e => { if (e.key === 'Escape') { e.stopPropagation(); close(); } };
  const closeBtn = el('button', { class: 'icon-button', 'aria-label': 'Close menu' }, icon('window.close', 16));
  closeBtn.addEventListener('click', close);
  drawer.append(el('div', { class: 'drawer-head' }, el('span', {}, title), closeBtn));

  const renderItems = (items, depth) => {
    const list = el('div', { class: 'drawer-list', role: 'group' });
    for (const it of items) {
      if (it === '-') { list.append(el('div', { class: 'menu-sep', role: 'separator' })); continue; }
      const label = typeof it.label === 'function' ? it.label() : it.label;
      const enabled = it.enabled ? !!it.enabled() : true;
      const checked = it.checked ? !!it.checked() : null;
      const lead = el('span', { class: 'icon-slot' }, checked === true ? el('span', { class: 'check-mark' }, '✓') : it.icon ? icon(it.icon, 18) : null);
      if (it.submenu) {
        const sec = el('details', { class: 'drawer-sub' });
        const sum = el('summary', { class: 'drawer-item' }, lead, el('span', { class: 'label' }, label));
        sec.append(sum);
        // Filled in when opened: submenus can be dynamic (recent files, layers…).
        sec.addEventListener('toggle', () => {
          if (sec.open && sec.children.length === 1) sec.append(renderItems(it.submenu(), depth + 1));
        });
        list.append(sec);
        continue;
      }
      const row = el('button', {
        class: 'drawer-item', disabled: !enabled,
        ...(checked !== null ? { role: 'menuitemcheckbox', 'aria-checked': String(checked) } : { role: 'menuitem' }),
      }, lead, el('span', { class: 'label' }, label));
      row.addEventListener('click', () => { close(); it.action(); });
      list.append(row);
    }
    return list;
  };
  for (const m of menus) {
    const sec = el('details', { class: 'drawer-top' });
    sec.append(el('summary', { class: 'drawer-item drawer-heading' }, el('span', { class: 'label' }, m.label)));
    sec.addEventListener('toggle', () => {
      if (sec.open) {
        // One top-level menu open at a time keeps the list short.
        for (const o of drawer.querySelectorAll('.drawer-top[open]')) if (o !== sec) o.open = false;
        sec.querySelector(':scope > .drawer-list')?.remove();
        sec.append(renderItems(m.items, 1)); // fresh: enabled/checked states change
      }
    });
    drawer.append(sec);
  }
  backdrop.addEventListener('pointerdown', e => { if (e.target === backdrop) close(); });
  backdrop.append(drawer);
  document.body.append(backdrop);
  document.addEventListener('keydown', onKey, true);
  drawer.querySelector('summary')?.focus();
  return close;
}

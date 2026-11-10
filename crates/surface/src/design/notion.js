// The Notion tab's handlers on the web, as notion.rs's are in the window: a drag picks words in one line and opens the
// toolbar, its Comment opens a thread beside the line, a post marks the words, a thread's click reopens it for a reply,
// « Add comment » writes on the page, a comment's « ... » opens its menu, and a '/' in the last line opens the blocks.
(() => {
  const root = document.querySelector('.no'), proto = document.getElementById('no-proto');
  if (!root || !proto) return;
  // The protos' ids wait in data-id, so each is in the page once: on the clone that is shown.
  proto.querySelectorAll('[id]').forEach(e => { e.dataset.id = e.id; e.removeAttribute('id'); });
  const part = sel => {
    const c = proto.querySelector(sel).cloneNode(true);
    [c, ...c.querySelectorAll('[data-id]')].forEach(e => { if (e.dataset.id) e.id = e.dataset.id; });
    return c;
  };
  const texts = [...root.querySelectorAll('[id^="no-text-"]')].map(e => e.textContent);
  const same = (p, q) => !!p && !!q && p.line === q.line && p.start === q.start && p.end === q.end;
  const words = p => texts[p.line].slice(p.start, p.end);
  // draft: the open thread's field, kept across a redraw and emptied when another thread opens, as notion.rs's signal.
  let picked = null, toolbar = false, composing = null, draft = '';
  const threads = [];
  const pending = () => picked || (threads.some(t => same(t.p, composing)) ? null : composing);

  // What the browser holds selected, when it is one run inside one line, without its outer spaces.
  const selected = () => {
    const s = getSelection();
    if (!s.rangeCount || s.isCollapsed) return null;
    const r = s.getRangeAt(0);
    const text = n => (n.nodeType === 1 ? n : n.parentElement).closest('[id^="no-text-"]');
    const el = text(r.startContainer);
    if (!el || el !== text(r.endContainer)) return null;
    const before = document.createRange();
    before.setStart(el, 0);
    before.setEnd(r.startContainer, r.startOffset);
    const all = r.toString();
    const start = before.toString().length + all.length - all.trimStart().length;
    const end = start + all.trim().length;
    return start < end ? { line: Number(el.id.slice(8)), start, end } : null;
  };

  // A line's words cut wherever a phrase starts or ends, as notion.rs's runs.
  const runs = l => {
    const w = texts[l], p = pending(), on = threads.map(t => t.p).concat(p ? [p] : []).filter(q => q.line === l);
    const cuts = [...new Set([0, w.length, ...on.flatMap(q => [q.start, q.end])])].sort((a, b) => a - b);
    const covers = (q, at) => !!q && q.line === l && q.start <= at && at < q.end;
    const el = document.getElementById('no-text-' + l);
    el.textContent = '';
    for (let i = 0; i + 1 < cuts.length; i++) {
      const span = document.createElement('span'), at = cuts[i];
      span.className = covers(p, at) ? 'no-pick' : threads.some(t => covers(t.p, at)) ? 'no-mark' : '';
      span.textContent = w.slice(at, cuts[i + 1]);
      el.append(span);
    }
  };

  const said = (id, text) => {
    const c = part('.no-comment');
    c.id = id;
    c.querySelector('.no-words').textContent = text;
    c.querySelector('.no-more').id = id + '-more';
    return c;
  };
  const reply = hint => {
    const r = part('.no-thread .no-reply');
    const hold = r.querySelector('.ds-placeholder');
    hold.textContent = hint;
    hold.hidden = !!draft;
    r.querySelector('input').value = draft;
    r.querySelector('.no-send').classList.toggle('off', !draft);
    return r;
  };
  const thread = (id, p) => {
    const t = part('.no-thread');
    t.id = id;
    t.querySelector('.no-reply').remove();
    t.querySelector('.no-quote').textContent = words(p);
    return t;
  };

  // The whole Notion state drawn again; `focus` puts the caret in the open thread's field.
  const draw = focus => {
    texts.forEach((_, l) => runs(l));
    document.getElementById('no-tool')?.remove();
    if (toolbar && picked) {
      document.getElementById('no-line-' + picked.line).append(part('.no-tool'));
    }
    root.querySelectorAll('.no-threads').forEach(e => e.remove());
    texts.forEach((_, l) => {
      const fresh = composing && composing.line === l && !threads.some(t => same(t.p, composing));
      const mine = threads.map((t, i) => [t, i]).filter(([t]) => t.p.line === l);
      if (!mine.length && !fresh) return;
      const box = document.createElement('div');
      box.className = 'no-threads';
      for (const [t, i] of mine) {
        const el = thread('no-thread-' + i, t.p);
        // Folded, as notion.rs: the first and the last, the replies between behind « Show n replies ».
        const n = t.said.length, last = n - 1;
        if (t.folded && n > 2) {
          const fold = document.createElement('div');
          fold.id = 'no-unfold-' + i;
          fold.className = 'no-fold';
          fold.textContent = n === 3 ? 'Show 1 reply' : `Show ${n - 2} replies`;
          el.append(said(`no-said-${i}-0`, t.said[0]), fold, said(`no-said-${i}-${last}`, t.said[last]));
        } else t.said.forEach((w, k) => el.append(said(`no-said-${i}-${k}`, w)));
        if (same(composing, t.p)) el.append(reply('Reply...'));
        box.append(el);
      }
      if (fresh) {
        const el = thread('no-new', composing);
        el.append(reply('Add a comment...'));
        box.append(el);
      }
      document.getElementById('no-line-' + l).append(box);
    });
    if (focus) document.getElementById('no-input')?.focus();
  };

  const post = () => {
    const input = document.getElementById('no-input');
    if (!composing || !input?.value) return;
    const t = threads.find(t => same(t.p, composing));
    if (t) t.said.push(input.value);
    else threads.push({ p: composing, said: [input.value], folded: false });
    draft = '';
    draw(true);
  };
  const pagePost = () => {
    const input = document.getElementById('no-page-input');
    if (!input.value) return;
    const page = document.getElementById('no-page');
    page.insertBefore(said('no-page-' + page.querySelectorAll(':scope > .no-comment').length, input.value), page.querySelector(':scope > .no-reply'));
    input.value = '';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  };

  // A press closes the toolbar unless it is on it; the release picks, as the window's.
  root.addEventListener('mousedown', e => {
    if (e.button !== 0) return;
    if (e.target.closest('.no-tool, .no-slash')) { e.preventDefault(); return; }
    toolbar = false;
    document.getElementById('no-tool')?.remove();
  });
  root.addEventListener('mouseup', e => {
    if (e.button !== 0 || e.target.closest('.no-tool, .no-slash, .no-threads, .no-page, .no-block')) return;
    const now = selected();
    if (!now && !picked) return;
    picked = now;
    toolbar = !!now;
    if (now) getSelection().removeAllRanges();
    draw();
  });
  root.addEventListener('click', e => {
    const at = sel => e.target.closest(sel);
    if (at('#no-comment')) {
      composing = picked;
      draft = '';
      picked = null;
      toolbar = false;
      draw(true);
    } else if (at('#no-send')) post();
    else if (at('#no-page-post')) pagePost();
    else if (at('.no-more')) {
      const c = at('.no-comment'), was = c.classList.contains('menu');
      root.querySelectorAll('.no-comment.menu').forEach(m => { m.classList.remove('menu'); m.querySelector('.no-menu')?.remove(); });
      if (!was) {
        const menu = part('.no-menu');
        menu.id = 'no-menu';
        c.classList.add('menu');
        c.append(menu);
      }
    } else if (at('.no-menu')) return;
    else if (at('#no-add')) {
      const page = document.getElementById('no-page');
      page.hidden = false;
      composing = null;
      draft = '';
      threads.forEach(t => { t.folded = true; });
      draw();
      document.getElementById('no-page-input').focus();
    } else if (at('[data-kind]')) pick(at('[data-kind]').dataset.kind);
    else if (at('.no-fold')) {
      threads[Number(at('.no-fold').id.slice(10))].folded = false;
      draw();
    } else if (at('.no-thread[id^="no-thread-"]')) {
      const t = threads[Number(at('.no-thread').id.slice(10))];
      if (!same(composing, t.p)) { composing = t.p; draft = ''; draw(true); }
    }
  });

  // The page's « ... », DropdownMenu: its trigger flips the states notion.rs's signal draws, any other click or Escape closes it.
  // Both items are links, each a download: Export PDF dash's printed document, Export Markdown the document's text.
  const pageMenu = document.getElementById('no-page-menu');
  const trigger = document.getElementById('no-page-menu-trigger'), content = document.getElementById('no-page-menu-content');
  const openMenu = open => {
    [pageMenu, trigger, content].forEach(e => { e.dataset.state = open ? 'open' : 'closed'; });
    trigger.setAttribute('aria-expanded', open);
    content.hidden = !open;
  };
  document.addEventListener('click', e => {
    if (e.target.closest('#no-page-menu-trigger')) return openMenu(content.hidden);
    if (content.hidden || (e.target.closest('#no-page-menu-content') && !e.target.closest('.dropdown-menu-item'))) return;
    openMenu(false);
  });
  document.addEventListener('keydown', e => { if (e.key === 'Escape' && !content.hidden) openMenu(false); });

  // The slash menu: a '/' opens it, what follows filters it, Enter or a click picks, Escape closes.
  const block = document.getElementById('no-block'), menu = document.getElementById('no-slash');
  const slashInput = document.getElementById('no-slash-input');
  const items = () => [...menu.querySelectorAll('[data-kind]')];
  const pick = kind => {
    block.className = 'no-line no-block ' + kind;
    slashInput.value = '';
    slashInput.dispatchEvent(new Event('input', { bubbles: true }));
    slashInput.focus();
  };
  root.addEventListener('input', e => {
    const input = e.target;
    const holder = input.closest('.ds-holder');
    holder?.querySelector('.ds-placeholder')?.toggleAttribute('hidden', !!input.value);
    input.closest('.no-reply')?.querySelector('.no-send')?.classList.toggle('off', !input.value);
    if (input.id === 'no-input') draft = input.value;
    if (input !== slashInput) return;
    const v = input.value, rest = v.startsWith('/') ? v.slice(1).toLowerCase() : null;
    menu.hidden = rest === null;
    for (const item of items()) item.hidden = rest !== null && !item.textContent.toLowerCase().includes(rest);
  });
  root.addEventListener('keydown', e => {
    if (e.key !== 'Enter' && e.key !== 'Escape') return;
    if (e.target === slashInput && !menu.hidden) {
      e.preventDefault();
      const first = items().find(i => !i.hidden);
      // Enter with no block offered keeps what is typed; only Escape drops it.
      if (e.key === 'Enter') { if (first) pick(first.dataset.kind); }
      else { slashInput.value = ''; slashInput.dispatchEvent(new Event('input', { bubbles: true })); }
    } else if (e.key === 'Enter' && e.target.id === 'no-input') post();
    else if (e.key === 'Enter' && e.target.id === 'no-page-input') pagePost();
  });
})();

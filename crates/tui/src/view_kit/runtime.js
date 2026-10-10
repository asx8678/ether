// New in ether: the view kit runtime. ether runs this in the view page through the DevTools pipe
// after the model's HTML has loaded (the page's CSP still blocks every script the model writes).
// It expands the components, draws the connectors, checks the result, and returns the drawn
// size: { width, height, issues }.
(() => {
  const issues = [];
  const SVG = 'http://www.w3.org/2000/svg';
  const fmt = new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 });
  const num = (value) => { const n = parseFloat(value); return Number.isFinite(n) ? n : 0; };
  const tone = (name) => `var(--${(name || 'accent').replace(/^tone-/, '')})`;
  const el = (tag, cls, text) => {
    const node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text != null) node.textContent = text;
    return node;
  };
  const bar = (fraction, color) => {
    const track = el('div', 'x-bar-track');
    const fill = track.appendChild(el('div', 'x-bar-fill'));
    fill.style.width = `${Math.max(0.6, Math.min(100, fraction * 100))}%`;
    fill.style.background = color;
    return track;
  };

  // <x-bars unit max tone><x-bar label value display tone></x-bar>…</x-bars>
  for (const bars of document.querySelectorAll('x-bars')) {
    const items = [...bars.querySelectorAll('x-bar')].map((item) => ({
      label: item.getAttribute('label') ?? item.textContent.trim(),
      value: num(item.getAttribute('value')),
      display: item.getAttribute('display'),
      tone: item.getAttribute('tone') || bars.getAttribute('tone'),
    }));
    const max = num(bars.getAttribute('max')) || Math.max(0, ...items.map((i) => i.value)) || 1;
    const unit = bars.getAttribute('unit');
    const grid = el('div', 'x-bars');
    for (const item of items) {
      grid.append(el('span', 'x-bar-label', item.label), bar(item.value / max, tone(item.tone)),
        el('span', 'x-bar-value', item.display || fmt.format(item.value) + (unit ? ` ${unit}` : '')));
    }
    bars.replaceWith(grid);
  }

  // <x-gauge label value max tone type="ring|bar">
  for (const gauge of document.querySelectorAll('x-gauge')) {
    const max = num(gauge.getAttribute('max')) || 100;
    const ratio = Math.max(0, Math.min(1, num(gauge.getAttribute('value')) / max));
    const pct = `${Math.round(ratio * 100)}%`;
    const label = gauge.getAttribute('label') || '';
    const color = tone(gauge.getAttribute('tone'));
    let node;
    if (gauge.getAttribute('type') === 'bar') {
      node = el('div', 'x-bars');
      node.append(el('span', 'x-bar-label', label), bar(ratio, color), el('span', 'x-bar-value', pct));
    } else {
      node = el('div', 'x-gauge');
      const r = 15.9, c = 2 * Math.PI * r;
      node.innerHTML = `<svg viewBox="0 0 42 42" width="92" height="92"><circle cx="21" cy="21" r="${r}" fill="none" stroke="var(--track)" stroke-width="4.2"/><circle cx="21" cy="21" r="${r}" fill="none" stroke="${color}" stroke-width="4.2" stroke-linecap="round" stroke-dasharray="${c * ratio} ${c}" transform="rotate(-90 21 21)"/><text x="21" y="23.6" text-anchor="middle" font-size="8.5" font-weight="700" fill="var(--fg)">${pct}</text></svg>`;
      node.append(el('div', 'x-gauge-label', label));
    }
    gauge.replaceWith(node);
  }

  // <x-spark values="3 5 2 8" tone>
  for (const spark of document.querySelectorAll('x-spark')) {
    const values = (spark.getAttribute('values') || '').split(/[\s,]+/).filter(Boolean).map(num);
    const node = el('span', 'x-spark');
    if (values.length > 1) {
      const w = 160, h = 30, max = Math.max(...values), min = Math.min(...values), span = max - min || 1;
      const points = values.map((v, i) => `${(i * w) / (values.length - 1)},${h - 2 - ((v - min) * (h - 4)) / span}`).join(' ');
      const color = tone(spark.getAttribute('tone'));
      node.innerHTML = `<svg width="${w}" height="${h}" viewBox="0 0 ${w} ${h}"><polyline points="0,${h} ${points} ${w},${h}" fill="${color}" fill-opacity=".14" stroke="none"/><polyline points="${points}" fill="none" stroke="${color}" stroke-width="2" stroke-linejoin="round"/></svg>`;
    }
    spark.replaceWith(node);
  }

  // <x-table>| a | b |\n|---|---|\n| 1 | 2 |</x-table>: a markdown table, styled, with numeric
  // columns right-aligned (about half the tokens of <table> markup).
  for (const source of document.querySelectorAll('x-table')) {
    const rows = source.textContent.split('\n').map((line) => line.trim()).filter((line) => line.includes('|'))
      .filter((line) => !/^\|?\s*:?-{2,}/.test(line))
      .map((line) => line.replace(/^\|/, '').replace(/\|$/, '').split('|').map((cell) => cell.trim()));
    const table = el('table');
    if (rows.length) {
      const numeric = rows[0].map((_, col) => rows.length > 1 && rows.slice(1).every((row) => /^[-+]?[\d.,]+\s*[%kKMGTB]*\w{0,3}$/.test(row[col] ?? '')));
      const head = table.appendChild(el('thead')).appendChild(el('tr'));
      rows[0].forEach((cell, col) => { const th = head.appendChild(el('th', numeric[col] ? 'num' : null, cell)); });
      const body = table.appendChild(el('tbody'));
      for (const row of rows.slice(1)) {
        const tr = body.appendChild(el('tr'));
        rows[0].forEach((_, col) => tr.appendChild(el('td', numeric[col] ? 'num' : null, row[col] ?? '')));
      }
    }
    source.replaceWith(table);
  }

  // <x-stat label="build" value="passing" tone="ok" hint="…">: a status tile.
  for (const stat of document.querySelectorAll('x-stat')) {
    const tile = el('div', 'x-stat');
    tile.style.setProperty('--tone', tone(stat.getAttribute('tone') || 'fg'));
    tile.append(el('div', 'x-stat-label', stat.getAttribute('label') || ''), el('div', 'x-stat-value', stat.getAttribute('value') ?? stat.textContent.trim()));
    if (stat.getAttribute('hint')) tile.append(el('div', 'x-stat-hint', stat.getAttribute('hint')));
    stat.replaceWith(tile);
  }

  // <x-edge from to label tone dashed both>: arrows between elements the model laid out.
  const edges = [];
  for (const edge of document.querySelectorAll('x-edge')) {
    const from = document.getElementById(edge.getAttribute('from') || '');
    const to = document.getElementById(edge.getAttribute('to') || '');
    if (!from || !to || from === to) {
      issues.push(`x-edge from="${edge.getAttribute('from')}" to="${edge.getAttribute('to')}": ${!from || !to ? 'no element with that id' : 'connects an element to itself'}`);
    } else {
      edges.push({ from, to, label: edge.getAttribute('label') || '', tone: edge.getAttribute('tone') || 'accent', dashed: edge.hasAttribute('dashed'), both: edge.hasAttribute('both') });
    }
    edge.remove();
  }
  if (edges.length) drawEdges(edges);

  function drawEdges(edges) {
    // Room for arrows: the container that separates two connected boxes gets gutters wide
    // enough for an arrow and its label, whatever gaps the model chose.
    for (const edge of edges) {
      const ancestors = new Set();
      for (let node = edge.from; node; node = node.parentElement) ancestors.add(node);
      let common = edge.to;
      while (common && !ancestors.has(common)) common = common.parentElement;
      if (!common || common === document.body) common = edge.from.parentElement;
      const style = getComputedStyle(common);
      if (/grid|flex/.test(style.display)) {
        if (num(style.columnGap) < 64) common.style.columnGap = '64px';
        if (num(style.rowGap) < 44) common.style.rowGap = '44px';
      }
    }
    document.body.style.position = 'relative';
    const origin = document.body.getBoundingClientRect();
    const box = (node) => {
      const r = node.getBoundingClientRect();
      return { l: r.left - origin.left, t: r.top - origin.top, r: r.right - origin.left, b: r.bottom - origin.top, node };
    };
    const center = (b) => [(b.l + b.r) / 2, (b.t + b.b) / 2];

    // Sides: facing sides of the two boxes, horizontal when they share rows.
    for (const edge of edges) {
      const a = box(edge.from), b = box(edge.to);
      const [ax, ay] = center(a), [bx, by] = center(b);
      const shareRows = Math.min(a.b, b.b) - Math.max(a.t, b.t) > 0;
      const shareCols = Math.min(a.r, b.r) - Math.max(a.l, b.l) > 0;
      const horizontal = shareRows || (!shareCols && Math.abs(bx - ax) >= Math.abs(by - ay));
      edge.a = a; edge.b = b;
      edge.sideA = horizontal ? (bx > ax ? 'r' : 'l') : (by > ay ? 'b' : 't');
      edge.sideB = { r: 'l', l: 'r', b: 't', t: 'b' }[edge.sideA];
    }

    // Ports: several arrows on one side of a box spread along it, ordered by where their
    // other end is, so parallel and fanned-out arrows never share a line.
    const ports = new Map();
    for (const edge of edges) {
      for (const [end, side, other] of [['A', edge.sideA, edge.b], ['B', edge.sideB, edge.a]]) {
        const node = end === 'A' ? edge.from : edge.to;
        const key = node;
        if (!ports.has(key)) ports.set(key, []);
        ports.get(key).push({ edge, end, side, other });
      }
    }
    for (const [node, list] of ports) {
      const b = box(node);
      for (const side of ['l', 'r', 't', 'b']) {
        const group = list.filter((p) => p.side === side);
        const vertical = side === 'l' || side === 'r';
        group.sort((p, q) => (vertical ? center(p.other)[1] - center(q.other)[1] : center(p.other)[0] - center(q.other)[0]));
        group.forEach((p, i) => {
          const t = group.length === 1 ? 0.5 : 0.2 + (0.6 * i) / (group.length - 1);
          const point = vertical ? [side === 'r' ? b.r : b.l, b.t + (b.b - b.t) * t] : [b.l + (b.r - b.l) * t, side === 'b' ? b.b : b.t];
          p.edge[`p${p.end}`] = point;
        });
      }
    }

    const svg = document.createElementNS(SVG, 'svg');
    svg.setAttribute('class', 'x-edges');
    const defs = svg.appendChild(document.createElementNS(SVG, 'defs'));
    const markers = new Set();
    const normal = { r: [1, 0], l: [-1, 0], b: [0, 1], t: [0, -1] };
    for (const edge of edges) {
      const p0 = edge.pA, p3 = edge.pB;
      const n0 = normal[edge.sideA], n3 = normal[edge.sideB];
      const reach = Math.max(24, (Math.abs(p3[0] - p0[0]) * Math.abs(n0[0]) + Math.abs(p3[1] - p0[1]) * Math.abs(n0[1])) / 2);
      const p1 = [p0[0] + n0[0] * reach, p0[1] + n0[1] * reach];
      const p2 = [p3[0] + n3[0] * reach, p3[1] + n3[1] * reach];
      edge.curve = [p0, p1, p2, p3];
      const marker = `x-arrow-${edge.tone}`;
      if (!markers.has(marker)) {
        markers.add(marker);
        const m = defs.appendChild(document.createElementNS(SVG, 'marker'));
        for (const [k, v] of Object.entries({ id: marker, viewBox: '0 0 10 10', refX: '9', refY: '5', markerWidth: '7', markerHeight: '7', orient: 'auto-start-reverse' })) m.setAttribute(k, v);
        const head = m.appendChild(document.createElementNS(SVG, 'path'));
        head.setAttribute('d', 'M0,0 L10,5 L0,10 z');
        head.setAttribute('fill', tone(edge.tone));
      }
      const path = svg.appendChild(document.createElementNS(SVG, 'path'));
      path.setAttribute('class', 'x-edge');
      path.setAttribute('d', `M${p0} C${p1} ${p2} ${p3}`);
      path.setAttribute('stroke', tone(edge.tone));
      path.setAttribute('marker-end', `url(#${marker})`);
      if (edge.both) path.setAttribute('marker-start', `url(#${marker})`);
      if (edge.dashed) path.setAttribute('stroke-dasharray', '5 4');
    }
    document.body.prepend(svg);

    // Labels: beside the line, at the first spot that hits no box and no other label.
    const at = (curve, t) => [0, 1].map((i) => (1 - t) ** 3 * curve[0][i] + 3 * (1 - t) ** 2 * t * curve[1][i] + 3 * (1 - t) * t ** 2 * curve[2][i] + t ** 3 * curve[3][i]);
    const boxes = [...new Set([...edges.flatMap((e) => [e.from, e.to]), ...document.querySelectorAll('.card')])].map((node) => node.getBoundingClientRect());
    const placed = [];
    const hits = (r, list) => list.some((o) => r.left < o.right && r.right > o.left && r.top < o.bottom && r.bottom > o.top);
    for (const edge of edges.filter((e) => e.label)) {
      const label = document.body.appendChild(el('div', 'x-edge-label', edge.label));
      label.style.color = tone(edge.tone);
      const horizontal = edge.sideA === 'l' || edge.sideA === 'r';
      const offsets = horizontal ? ['translate(-50%, calc(-100% - 3px))', 'translate(-50%, 3px)'] : ['translate(7px, -50%)', 'translate(calc(-100% - 7px), -50%)'];
      let fits = false;
      for (const t of [0.5, 0.35, 0.65, 0.2, 0.8]) {
        for (const transform of offsets) {
          const [x, y] = at(edge.curve, t);
          label.style.left = `${x}px`; label.style.top = `${y}px`; label.style.transform = transform;
          const r = label.getBoundingClientRect();
          if (!hits(r, boxes) && !hits(r, placed)) { fits = true; break; }
        }
        if (fits) break;
      }
      if (!fits) {
        const [x, y] = at(edge.curve, 0.5);
        label.style.left = `${x}px`; label.style.top = `${y}px`; label.style.transform = offsets[0];
        issues.push(`connector label "${edge.label}" has no free spot; put more space between the boxes it joins`);
      }
      placed.push(label.getBoundingClientRect());
    }

    // Check: arrows that run through a third box.
    const name = (node) => node.id || node.querySelector('h1,h2,h3,h4')?.textContent.trim() || node.textContent.trim().slice(0, 24);
    const cards = [...new Set([...edges.flatMap((e) => [e.from, e.to]), ...document.querySelectorAll('.card')])];
    for (const edge of edges) {
      for (const card of cards) {
        if (card === edge.from || card === edge.to || card.contains(edge.from) || card.contains(edge.to) || edge.from.contains(card) || edge.to.contains(card)) continue;
        const r = box(card);
        const crosses = Array.from({ length: 23 }, (_, i) => at(edge.curve, (i + 1) / 24)).some(([x, y]) => x > r.l + 2 && x < r.r - 2 && y > r.t + 2 && y < r.b - 2);
        if (crosses) issues.push(`the arrow ${name(edge.from)} → ${name(edge.to)} runs through ${name(card)}; move the boxes so it has a clear path`);
      }
    }
    svg.setAttribute('width', Math.ceil(document.body.scrollWidth));
    svg.setAttribute('height', Math.ceil(document.body.scrollHeight));
  }

  // Drawn size, and checks on the whole view.
  const fit = document.body.clientWidth;
  let right = 0, bottom = 0;
  for (const node of document.body.querySelectorAll('*')) {
    const r = node.getBoundingClientRect();
    if (r.width > 0 && r.height > 0) { right = Math.max(right, r.right); bottom = Math.max(bottom, r.bottom); }
  }
  if (right > fit + 2) issues.push(`the view is ${Math.round(right)}px wide but ${fit}px fit, so it is shown scaled down`);
  for (const node of document.body.querySelectorAll('*')) {
    if (node.closest('svg')) continue; // SVG font sizes are in user units
    const text = [...node.childNodes].some((child) => child.nodeType === 3 && child.textContent.trim());
    if (text && num(getComputedStyle(node).fontSize) < 10) { issues.push('some text is smaller than 10px'); break; }
  }
  return { width: Math.ceil(right) + 2, height: Math.ceil(bottom) + 2, issues };
})()

// The web's Stage pick (families.rs Stage): what the window's handlers do, since a server render runs none.
document.addEventListener('click', e => {
  const line = e.target.closest('.ds-toggle .ds-pop-line');
  if (!line) return;
  const toggle = line.closest('.ds-toggle');
  toggle.querySelector('summary').textContent = line.textContent;
  toggle.querySelectorAll('.ds-pop-line').forEach(l => l.classList.toggle('on', l === line));
  toggle.open = false;
});

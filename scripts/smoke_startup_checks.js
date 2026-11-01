// Included only in the disposable startup-smoke source copy, never in production.
(() => {
  window.__buntiniStartupSmoke = { state: 'running' };
  (async () => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    const check = (condition, message) => { if (!condition) throw new Error(message); };
    const boot = await invoke('bootstrap');
    check(boot.tasks.length === 0 && boot.roots.length === 0 && boot.memories.length === 0, 'Disposable bootstrap must contain no user records');
    check(boot.settings.apps.length === 0 && boot.settings.externalShares.length === 0, 'Disposable bootstrap must grant no personal app/data access');
    check(boot.diagnostics.cloudKeys.length === 0 && !boot.diagnostics.chatgptAccount, 'Disposable credential namespaces must be empty');
    const progress = await invoke('task_progress');
    check(progress.length === 0, 'Fresh profile must have no pending workflows');
    const position = await invoke('plugin:window|outer_position', { label: 'main' });
    const before = await invoke('plugin:window|outer_size', { label: 'main' });
    check(before.width > 0 && before.height > 0, 'Window must have a nonzero size');
    await invoke('plugin:window|set_size', { label: 'main', value: { Logical: { width: 420, height: 620 } } });
    await invoke('plugin:window|set_position', { label: 'main', value: { Physical: position } });
    await invoke('plugin:window|set_ignore_cursor_events', { label: 'main', value: false });
    const monitor = await invoke('plugin:window|current_monitor', { label: 'main' });
    const cursor = await invoke('plugin:window|cursor_position', {});
    check(!!monitor && Number.isFinite(cursor.x) && Number.isFinite(cursor.y), 'Window monitor/cursor APIs must be allowed');
    let ungrantedDenied = false;
    let denialReason = '';
    try { await invoke('plugin:window|set_title', { label: 'main', value: 'Unexpectedly allowed' }); }
    catch (error) { denialReason = String(error); ungrantedDenied = /not allowed|denied/i.test(denialReason); }
    check(ungrantedDenied, 'An ungranted window operation must be rejected');
    window.__buntiniStartupSmoke = {
      state: 'done', passed: true, bootstrap: true, emptyProfile: true,
      personalAppAccess: false, windowReadsAndMutations: true,
      ungrantedWindowOperationDenied: true,
      denialReason,
      frontendMounted: document.querySelector('#root')?.children.length > 0,
      onboardingVisible: !!document.querySelector('.setup-progress'),
      location: window.location.href,
    };
  })().catch(error => {
    window.__buntiniStartupSmoke = { state: 'done', passed: false, error: String(error) };
  });
})();

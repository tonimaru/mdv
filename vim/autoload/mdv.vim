" mdv.vim - Markdown Directory Viewer integration for Vim/Neovim
" Multi-workspace support

" Configuration defaults
if !exists('g:mdv_port')
  let g:mdv_port = 3000
endif

if !exists('g:mdv_host')
  let g:mdv_host = 'localhost'
endif

if !exists('g:mdv_sync_scroll')
  let g:mdv_sync_scroll = 1
endif

if !exists('g:mdv_auto_open_browser')
  let g:mdv_auto_open_browser = 1
endif

" Internal state
let s:mdv_job = v:null
let s:registered_workspaces = {}

" Get base URL
function! s:base_url() abort
  return 'http://' . g:mdv_host . ':' . g:mdv_port
endfunction

" Run command asynchronously (detached)
function! s:run_detached(cmd) abort
  if has('nvim')
    call jobstart(a:cmd, {'detach': v:true})
  elseif has('job')
    call job_start(a:cmd, {'stoponexit': ''})
  else
    silent execute '!' . join(a:cmd, ' ') . ' &'
  endif
endfunction

" Run command silently (for curl)
function! s:run_silent(cmd) abort
  if has('nvim')
    call jobstart(a:cmd)
  elseif has('job')
    call job_start(a:cmd)
  else
    silent execute '!' . join(a:cmd, ' ') . ' >/dev/null 2>&1 &'
  endif
endfunction

" Check if server is running
function! mdv#is_running() abort
  let l:url = s:base_url() . '/api/status'
  let l:result = system('curl -s -o /dev/null -w "%{http_code}" --max-time 1 ' . shellescape(l:url))
  return l:result ==# '200'
endfunction

" Start mdv server
function! mdv#start() abort
  if mdv#is_running()
    echo 'mdv server is already running on port ' . g:mdv_port
    return 1
  endif

  let l:cmd = ['mdv', '--port', string(g:mdv_port)]

  if has('nvim')
    let s:mdv_job = jobstart(l:cmd, {'detach': v:true})
  elseif has('job')
    let s:mdv_job = job_start(l:cmd, {'stoponexit': ''})
  else
    silent execute '!mdv --port ' . g:mdv_port . ' &'
  endif

  " Wait for server
  let l:retries = 20
  while l:retries > 0
    sleep 100m
    if mdv#is_running()
      echo 'mdv server started on port ' . g:mdv_port
      return 1
    endif
    let l:retries -= 1
  endwhile

  echoerr 'Failed to start mdv server'
  return 0
endfunction

" Stop mdv server
function! mdv#stop() abort
  if has('nvim') && s:mdv_job != v:null
    call jobstop(s:mdv_job)
    let s:mdv_job = v:null
  elseif has('job') && s:mdv_job != v:null
    call job_stop(s:mdv_job)
    let s:mdv_job = v:null
  else
    silent execute '!pkill -f "mdv.*--port ' . g:mdv_port . '" 2>/dev/null'
  endif
  let s:registered_workspaces = {}
  echo 'mdv server stopped'
endfunction

" Ensure server is running
function! s:ensure_server() abort
  if !mdv#is_running()
    return mdv#start()
  endif
  return 1
endfunction

" Get project root (looks for .git, .hg, or uses cwd)
function! s:get_project_root() abort
  let l:dir = expand('%:p:h')
  while l:dir !=# '/'
    if isdirectory(l:dir . '/.git') || isdirectory(l:dir . '/.hg')
      return l:dir
    endif
    let l:dir = fnamemodify(l:dir, ':h')
  endwhile
  return getcwd()
endfunction

" Register workspace with server
function! mdv#register_workspace(...) abort
  if !s:ensure_server()
    return ''
  endif

  let l:root = a:0 > 0 ? a:1 : s:get_project_root()

  " Skip if already registered
  if has_key(s:registered_workspaces, l:root)
    return s:registered_workspaces[l:root]
  endif

  let l:url = s:base_url() . '/api/workspace/register'
  let l:json = '{"path":"' . escape(l:root, '"') . '"}'

  let l:result = system('curl -s -X POST -H "Content-Type: application/json" -d ' . shellescape(l:json) . ' ' . shellescape(l:url))

  try
    " Parse JSON response
    if has('nvim')
      let l:resp = json_decode(l:result)
    else
      let l:resp = json_decode(l:result)
    endif

    if has_key(l:resp, 'id')
      let s:registered_workspaces[l:root] = l:resp
      return l:resp
    endif
  catch
  endtry

  return ''
endfunction

" Navigate browser to current file
function! mdv#active() abort
  if !s:ensure_server()
    return
  endif

  let l:path = expand('%:p')
  if l:path !~# '\.md$'
    return
  endif

  " Skip if file does not exist
  if !filereadable(l:path)
    return
  endif

  " Register workspace first
  let l:ws = mdv#register_workspace()
  if empty(l:ws)
    return
  endif

  let l:url = s:base_url() . '/api/active?path=' . l:path
  call s:run_silent(['curl', '-s', l:url])
endfunction

" Open current file in browser
function! mdv#open() abort
  if !s:ensure_server()
    return
  endif

  let l:path = expand('%:p')
  if l:path !~# '\.md$'
    echoerr 'Current file is not a markdown file'
    return
  endif

  " Register workspace
  let l:ws = mdv#register_workspace()
  if empty(l:ws)
    echoerr 'Failed to register workspace'
    return
  endif

  " Get relative path
  let l:root = s:get_project_root()
  let l:relative = substitute(l:path, '^' . escape(l:root, '/') . '/', '', '')
  let l:url = s:base_url() . l:ws.url . '/' . l:relative

  " Open in browser
  if has('mac') || has('macunix')
    call s:run_detached(['open', l:url])
  elseif has('unix')
    if executable('wslview')
      call s:run_detached(['wslview', l:url])
    elseif executable('xdg-open')
      call s:run_detached(['xdg-open', l:url])
    else
      echoerr 'No browser opener found'
      return
    endif
  elseif has('win32') || has('win64')
    call s:run_detached(['cmd', '/c', 'start', '', l:url])
  endif

  echo 'Opened in browser: ' . l:relative
endfunction

" Sync scroll position
function! mdv#sync_scroll() abort
  if !g:mdv_sync_scroll
    return
  endif

  if !mdv#is_running()
    return
  endif

  let l:path = expand('%:p')
  if l:path !~# '\.md$'
    return
  endif

  " Skip if file does not exist
  if !filereadable(l:path)
    return
  endif

  let l:total = line('$')
  if l:total <= 1
    return
  endif

  let l:current = line('w0')
  let l:percent = (l:current * 100) / l:total

  let l:url = s:base_url() . '/api/remote/scroll?percent=' . l:percent
  call s:run_silent(['curl', '-s', l:url])
endfunction

" Toggle scroll sync
function! mdv#toggle_scroll() abort
  let g:mdv_sync_scroll = !g:mdv_sync_scroll
  echo 'mdv scroll sync: ' . (g:mdv_sync_scroll ? 'ON' : 'OFF')
endfunction

" Show status
function! mdv#status() abort
  if !mdv#is_running()
    echo 'mdv server is not running'
    return
  endif

  let l:url = s:base_url() . '/api/status'
  let l:result = system('curl -s ' . shellescape(l:url))

  try
    let l:resp = json_decode(l:result)
    echo 'mdv server running at ' . s:base_url()
    echo 'Scroll sync: ' . (g:mdv_sync_scroll ? 'ON' : 'OFF')
    echo 'Workspaces:'
    for l:ws in l:resp.workspaces
      echo '  - ' . l:ws.name . ' (' . l:ws.path . ')'
    endfor
  catch
    echo 'mdv server running at ' . s:base_url()
  endtry
endfunction

" Auto-register on BufEnter for .md files
function! mdv#on_buf_enter() abort
  let l:path = expand('%:p')
  if l:path !~# '\.md$'
    return
  endif

  " Skip if file does not exist
  if !filereadable(l:path)
    return
  endif

  if mdv#is_running()
    call mdv#register_workspace()
    call mdv#active()
  endif
endfunction

" Add workspace (register with server)
function! mdv#workspace_add(...) abort
  if !s:ensure_server()
    return
  endif

  let l:path = a:0 > 0 ? a:1 : s:get_project_root()

  " Expand path
  let l:path = fnamemodify(l:path, ':p')
  if l:path[-1:] ==# '/'
    let l:path = l:path[:-2]
  endif

  let l:url = s:base_url() . '/api/workspace/register'
  let l:json = '{"path":"' . escape(l:path, '"') . '"}'

  let l:result = system('curl -s -X POST -H "Content-Type: application/json" -d ' . shellescape(l:json) . ' ' . shellescape(l:url))

  try
    let l:resp = json_decode(l:result)
    if has_key(l:resp, 'id')
      let s:registered_workspaces[l:path] = l:resp
      echo 'Workspace added: ' . l:resp.name . ' (' . l:resp.id . ')'
    elseif has_key(l:resp, 'error')
      echoerr 'Error: ' . l:resp.error
    endif
  catch
    echoerr 'Failed to add workspace'
  endtry
endfunction

" Remove workspace (unregister from server)
function! mdv#workspace_remove(...) abort
  if !mdv#is_running()
    echo 'mdv server is not running'
    return
  endif

  " Get workspace list
  let l:status_url = s:base_url() . '/api/status'
  let l:result = system('curl -s ' . shellescape(l:status_url))

  try
    let l:resp = json_decode(l:result)
    if empty(l:resp.workspaces)
      echo 'No workspaces registered'
      return
    endif

    let l:workspace_id = ''

    if a:0 > 0
      " Use provided workspace id
      let l:workspace_id = a:1
    else
      " Show list and let user choose
      let l:choices = []
      let l:idx = 1
      for l:ws in l:resp.workspaces
        call add(l:choices, l:idx . '. ' . l:ws.name . ' (' . l:ws.id . ') - ' . l:ws.path)
        let l:idx += 1
      endfor

      echo 'Select workspace to remove:'
      for l:choice in l:choices
        echo l:choice
      endfor

      let l:input = input('Enter number (or workspace id): ')
      if empty(l:input)
        return
      endif

      if l:input =~# '^\d\+$'
        let l:num = str2nr(l:input)
        if l:num >= 1 && l:num <= len(l:resp.workspaces)
          let l:workspace_id = l:resp.workspaces[l:num - 1].id
        else
          echoerr 'Invalid selection'
          return
        endif
      else
        let l:workspace_id = l:input
      endif
    endif

    " Delete workspace
    let l:delete_url = s:base_url() . '/api/workspace/' . l:workspace_id
    let l:del_result = system('curl -s -X DELETE ' . shellescape(l:delete_url))

    let l:del_resp = json_decode(l:del_result)
    if has_key(l:del_resp, 'status') && l:del_resp.status ==# 'ok'
      " Remove from local cache
      for [l:path, l:ws] in items(s:registered_workspaces)
        if l:ws.id ==# l:workspace_id
          call remove(s:registered_workspaces, l:path)
          break
        endif
      endfor
      echo 'Workspace removed: ' . l:workspace_id
    elseif has_key(l:del_resp, 'error')
      echoerr 'Error: ' . l:del_resp.error
    endif
  catch
    echoerr 'Failed to remove workspace'
  endtry
endfunction

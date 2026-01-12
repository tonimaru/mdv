" mdv.vim - Markdown Directory Viewer integration for Vim/Neovim
" Autoload functions

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

if !exists('g:mdv_auto_start')
  let g:mdv_auto_start = 0
endif

" Internal state
let s:mdv_job = v:null

" Run command asynchronously (Vim/Neovim compatible)
function! s:run_async(cmd) abort
  if has('nvim')
    call jobstart(a:cmd, {'detach': v:true})
  elseif has('job')
    call job_start(a:cmd, {'stoponexit': ''})
  else
    silent execute '!' . join(a:cmd, ' ') . ' &'
  endif
endfunction

" Run command and discard output (for curl)
function! s:run_silent(cmd) abort
  if has('nvim')
    call jobstart(a:cmd)
  elseif has('job')
    call job_start(a:cmd)
  else
    silent execute '!' . join(a:cmd, ' ') . ' >/dev/null 2>&1 &'
  endif
endfunction

" Get the base URL
function! s:base_url() abort
  return 'http://' . g:mdv_host . ':' . g:mdv_port
endfunction

" Check if mdv server is running
function! mdv#is_running() abort
  let l:url = s:base_url() . '/'
  let l:result = system('curl -s -o /dev/null -w "%{http_code}" --max-time 1 ' . shellescape(l:url))
  return l:result ==# '200'
endfunction

" Start mdv server
function! mdv#start(...) abort
  if mdv#is_running()
    echo 'mdv is already running on port ' . g:mdv_port
    return
  endif

  let l:root = a:0 > 0 ? a:1 : getcwd()
  let l:cmd = ['mdv', l:root, '--port', string(g:mdv_port)]

  if has('nvim')
    let s:mdv_job = jobstart(l:cmd, {'detach': v:true})
  elseif has('job')
    let s:mdv_job = job_start(l:cmd, {'stoponexit': ''})
  else
    silent execute '!mdv ' . shellescape(l:root) . ' --port ' . g:mdv_port . ' &'
  endif

  " Wait for server to start
  let l:retries = 10
  while l:retries > 0
    sleep 100m
    if mdv#is_running()
      echo 'mdv started on port ' . g:mdv_port
      return
    endif
    let l:retries -= 1
  endwhile

  echoerr 'Failed to start mdv'
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
  echo 'mdv stopped'
endfunction

" Open current file in browser
function! mdv#open() abort
  " Start server if not running and auto_start is enabled
  if g:mdv_auto_start && !mdv#is_running()
    call mdv#start()
  endif

  if !mdv#is_running()
    echoerr 'mdv is not running. Use :MdvStart to start the server.'
    return
  endif

  let l:path = expand('%:.')
  if l:path !~# '\.md$'
    echoerr 'Current file is not a markdown file'
    return
  endif

  let l:url = s:base_url() . '/' . l:path

  " Open in browser (OS-specific)
  if has('mac') || has('macunix')
    call s:run_async(['open', l:url])
  elseif has('unix')
    if executable('xdg-open')
      call s:run_async(['xdg-open', l:url])
    elseif executable('wslview')
      call s:run_async(['wslview', l:url])
    else
      echoerr 'No browser opener found (xdg-open or wslview)'
      return
    endif
  elseif has('win32') || has('win64')
    call s:run_async(['cmd', '/c', 'start', '', l:url])
  endif

  echo 'Opened ' . l:path . ' in browser'
endfunction

" Navigate browser to current file
function! mdv#sync_page() abort
  if !mdv#is_running()
    return
  endif

  let l:path = expand('%:.')
  if l:path !~# '\.md$'
    return
  endif

  let l:url = s:base_url() . '/api/remote/navigate?path=' . l:path
  call s:run_silent(['curl', '-s', l:url])
endfunction

" Sync scroll position to browser
function! mdv#sync_scroll() abort
  if !g:mdv_sync_scroll
    return
  endif

  if !mdv#is_running()
    return
  endif

  let l:path = expand('%:.')
  if l:path !~# '\.md$'
    return
  endif

  " Calculate scroll percentage
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

" Get status
function! mdv#status() abort
  if mdv#is_running()
    echo 'mdv is running on ' . s:base_url()
  else
    echo 'mdv is not running'
  endif
  echo 'Scroll sync: ' . (g:mdv_sync_scroll ? 'ON' : 'OFF')
endfunction

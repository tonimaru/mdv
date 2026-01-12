" mdv.vim - Markdown Directory Viewer integration for Vim/Neovim
" Plugin entry point

if exists('g:loaded_mdv')
  finish
endif
let g:loaded_mdv = 1

" Commands
command! MdvStart call mdv#start()
command! MdvStop call mdv#stop()
command! MdvOpen call mdv#open()
command! MdvStatus call mdv#status()
command! MdvScrollToggle call mdv#toggle_scroll()

" Autocommands for markdown files
augroup mdv_sync
  autocmd!
  " Auto-register workspace and navigate on BufEnter
  autocmd BufEnter *.md call mdv#on_buf_enter()
  " Sync scroll on cursor movement
  autocmd CursorMoved,CursorMovedI *.md call mdv#sync_scroll()
augroup END

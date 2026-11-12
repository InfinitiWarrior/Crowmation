" Crowmation scene files (.cwm)
if exists("b:current_syntax")
  finish
endif

syn keyword cwmKeyword scene component import let repeat as wait at after with
syn keyword cwmSetting size fps background duration
syn keyword cwmObject rect circle arrow line image text code math model camera
syn keyword cwmAction move rotate scale fade fade_in fade_out color orbit tilt explode
syn keyword cwmProperty to by ease from width head radius corner outline outline_width opacity rotation lang content fov z layer pin
syn keyword cwmValue overlay world linear in out in_out
syn match cwmNumber "\<\d\+\(\.\d\+\)\=\(s\|ms\|deg\|rad\)\=\>"
syn match cwmColor "#\x\{6}\(\x\{2}\)\=\>"
syn match cwmEscape contained "\\[\\"n#]"
syn region cwmString start=+"+ skip=+\\\\\|\\"+ end=+"+ contains=cwmEscape
syn match cwmComment "//.*$"

hi def link cwmKeyword Keyword
hi def link cwmSetting PreProc
hi def link cwmObject Type
hi def link cwmAction Function
hi def link cwmProperty Identifier
hi def link cwmValue Constant
hi def link cwmNumber Number
hi def link cwmColor Constant
hi def link cwmEscape SpecialChar
hi def link cwmString String
hi def link cwmComment Comment

let b:current_syntax = "cwm"

Source tapes/common.tape
Output shots/install.txt

# Launch hidden; the addon list scans the fixture on start.
Hide
Type "clear && ./lemonup"
Enter
Sleep 4s
# i opens "Install an addon"; the box takes typing straight away.
Type "i"
Sleep 800ms
Type "https://github.com/BigWigsMods/LittleWigs"
Sleep 300ms
# enter on a link only asks "Install from a link?". The tape never answers
# it: esc is "not now", a second esc closes Install.
Enter
Show
Sleep 1500ms
Screenshot shots/install.png

Hide
Escape
Sleep 300ms
Escape
Sleep 300ms
Type "q"
Sleep 1s

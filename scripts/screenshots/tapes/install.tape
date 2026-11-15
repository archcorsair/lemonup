Source tapes/common.tape
Output shots/install.txt

# Launch hidden; the dashboard scans the fixture on start.
Hide
Type "clear && ./lemonup"
Enter
Sleep 4s
Type "i"
Sleep 800ms
Type "https://github.com/BigWigsMods/LittleWigs"
Sleep 300ms
# Enter only asks for confirmation; the tape never confirms.
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

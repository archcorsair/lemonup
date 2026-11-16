Source tapes/common.tape
Output shots/settings.txt

# Launch hidden; the addon list scans the fixture on start.
Hide
Type "clear && ./lemonup"
Enter
Sleep 4s
# s opens Settings (, does too).
Type "s"
Show
Sleep 1500ms
Screenshot shots/settings.png

Hide
Escape
Type "q"
Sleep 1s

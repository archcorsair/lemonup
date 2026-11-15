Source tapes/common.tape
Output shots/help.txt

# Launch hidden; the dashboard scans the fixture on start.
Hide
Type "clear && ./lemonup"
Enter
Sleep 4s
Type "?"
Show
Sleep 1500ms
Screenshot shots/help.png

Hide
Escape
Type "q"
Sleep 1s

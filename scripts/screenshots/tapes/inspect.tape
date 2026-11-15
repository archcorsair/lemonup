Source tapes/common.tape
Output shots/inspect.txt

# Launch hidden; the dashboard scans the fixture on start.
Hide
Type "clear && ./lemonup"
Enter
Sleep 4s
# HandyNotes (row 14): the provider lists v1.6.14, older than the installed
# v1.6.15, so inspect shows "Remote older".
Down 13
Enter
Show
Sleep 1500ms
Screenshot shots/inspect.png

Hide
Escape
Type "q"
Sleep 1s

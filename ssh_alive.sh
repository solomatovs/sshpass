#!/bin/bash
# count=0
# trap 'echo "Получен сигнал SIGINT (нажатие Ctrl+C), завершение сценария"; exit 1' SIGINT
# while true ; do
#     sleep 1
#     (( count++ ))
#     echo $count
# done

trap '' SIGHUP SIGINT SIGQUIT SIGTERM
sleep 60
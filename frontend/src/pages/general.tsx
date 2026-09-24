import { Button, buttonVariants } from "@/components/ui/button";
import { RefreshCcw, ExternalLink, GraduationCap, Trash2 } from "lucide-react";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select"
import {
    AlertDialog,
    AlertDialogAction,
    AlertDialogCancel,
    AlertDialogContent,
    AlertDialogDescription,
    AlertDialogFooter,
    AlertDialogHeader,
    AlertDialogTitle,
    AlertDialogTrigger,
} from "@/components/ui/alert-dialog"
import { useEffect, useState } from "react";
import { toast } from "sonner"
import { invoke } from '@tauri-apps/api/core';

// 値は変換エンジンの LearningType と同じ名前（settings.json の learning.mode）
type LearningMode = "inputAndOutput" | "onlyOutput" | "nothing";

const learningModes: { value: LearningMode; name: string; description: string }[] = [
    { value: "inputAndOutput", name: "学習する", description: "確定した候補を覚えて、次から上位に出します" },
    { value: "onlyOutput", name: "新しく学習しない", description: "これまでの学習は使い、新しくは覚えません" },
    { value: "nothing", name: "学習しない", description: "学習を使わず、新しくも覚えません" },
];

const Learning = () => {
    const [mode, setMode] = useState<LearningMode>("inputAndOutput");

    useEffect(() => {
        invoke<any>("get_config")
            .then((data) => {
                if (data.learning?.mode) {
                    setMode(data.learning.mode);
                }
            })
            .catch(() => {
                // Keep default values if config fetch fails
            });
    }, []);

    const handleModeChange = async (value: string) => {
        try {
            const data = await invoke<any>("get_config");
            data.learning = { ...data.learning, mode: value };
            await invoke("update_config", { newConfig: data });
            setMode(value as LearningMode);
        } catch (error) {
            toast("設定の更新に失敗しました");
        }
    };

    const handleReset = async () => {
        try {
            await invoke("reset_learning");
            toast("学習をリセットしました");
        } catch (error) {
            toast("学習のリセットに失敗しました");
        }
    };

    const current = learningModes.find((item) => item.value === mode);

    return (
        <section className="space-y-2">
            <h1 className="text-sm font-bold text-foreground">学習</h1>
            <div className="flex items-center space-x-4 rounded-md border p-4">
                <GraduationCap />
                <div className="flex-1 space-y-1">
                    <p id="learning-mode-label" className="text-sm font-medium leading-none">
                        変換の学習
                    </p>
                    <p className="text-xs text-muted-foreground">
                        {current?.description}
                    </p>
                </div>
                <Select value={mode} onValueChange={handleModeChange}>
                    <SelectTrigger className="w-48" aria-labelledby="learning-mode-label">
                        <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                        {learningModes.map((item) => (
                            <SelectItem key={item.value} value={item.value}>
                                {item.name}
                            </SelectItem>
                        ))}
                    </SelectContent>
                </Select>
            </div>
            <div className="flex items-center space-x-4 rounded-md border p-4">
                <Trash2 />
                <div className="flex-1 space-y-1">
                    <p className="text-sm font-medium leading-none">
                        学習をリセット
                    </p>
                    <p className="text-xs text-muted-foreground">
                        これまでに覚えた候補をすべて消します
                    </p>
                </div>
                <AlertDialog>
                    <AlertDialogTrigger asChild>
                        <Button variant="secondary">リセット</Button>
                    </AlertDialogTrigger>
                    <AlertDialogContent>
                        <AlertDialogHeader>
                            <AlertDialogTitle>学習をリセットしますか？</AlertDialogTitle>
                            <AlertDialogDescription>
                                これまでに覚えた候補がすべて消え、変換の順位が最初の状態に戻ります。元に戻すことはできません。
                            </AlertDialogDescription>
                        </AlertDialogHeader>
                        <AlertDialogFooter>
                            <AlertDialogCancel>キャンセル</AlertDialogCancel>
                            <AlertDialogAction
                                className={buttonVariants({ variant: "destructive" })}
                                onClick={handleReset}
                            >
                                リセットする
                            </AlertDialogAction>
                        </AlertDialogFooter>
                    </AlertDialogContent>
                </AlertDialog>
            </div>
        </section>
    )
}

export const General = () => {
    return (
        <div className="space-y-8">
            <section className="space-y-2">
                <h1 className="text-sm font-bold text-foreground">バージョンと更新プログラム</h1>
                <div className="flex items-center space-x-4 rounded-md border p-4">
                    <RefreshCcw />
                    <div className="flex-1 space-y-1">
                        <p className="text-sm font-medium leading-none">
                            v0.1.0-alpha.1
                        </p>
                    </div>
                    <Button  variant="secondary">
                        <a href="https://github.com/fkunn1326/azooKey-Windows/releases" className="flex items-center gap-x-2" target="_blank" rel="noopener noreferrer">
                            <ExternalLink />
                            更新を確認する
                        </a>
                    </Button>
                </div>
            </section>
            <Learning />
            {/* <section className="space-y-2">
                <h1 className="text-sm font-bold text-foreground">診断とフィードバック</h1>
                <div className="flex items-center space-x-4 rounded-md border p-4">
                    <FileChartColumn />
                    <div className="flex-1 space-y-1">
                        <p className="text-sm font-medium leading-none">
                            診断データ
                        </p>
                        <p className="text-xs text-muted-foreground">
                            診断データを保存し、バグの修正に役立てます
                        </p>
                    </div>
                    <Switch />
                </div>
            </section> */}
        </div>
    )
}